//! Provide `StateMachineStore`, an in-memory KV state machine implementation.

use std::collections::BTreeMap;
use std::io;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use futures::Stream;
use futures::TryStreamExt;
use futures::lock::Mutex;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;
use openraft::RaftTypeConfig;
use openraft::SnapshotMeta;
use openraft::StoredMembership;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use openraft::storage::Snapshot;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug)]
pub struct StoredSnapshot<C: RaftTypeConfig> {
    pub meta: SnapshotMeta<C>,

    /// The data of the state machine at the time of this snapshot.
    pub data: Vec<u8>,
}

/// Data contained in the Raft state machine.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct StateMachineData {
    /// Application data.
    pub data: BTreeMap<String, String>,
}

/// Inner storage for the state machine.
#[derive(Debug)]
pub struct StateMachineStoreInner<C: RaftTypeConfig> {
    pub last_applied_log: Option<LogId<C>>,

    pub last_membership: StoredMembership<C>,

    /// The Raft state machine.
    pub state_machine: StateMachineData,

    /// Used in identifier for snapshot.
    snapshot_idx: AtomicU64,

    /// The last received snapshot.
    pub current_snapshot: Option<StoredSnapshot<C>>,
}

impl<C: RaftTypeConfig> Default for StateMachineStoreInner<C> {
    fn default() -> Self {
        Self {
            last_applied_log: None,
            last_membership: StoredMembership::default(),
            state_machine: StateMachineData::default(),
            snapshot_idx: AtomicU64::new(0),
            current_snapshot: None,
        }
    }
}

impl<C: RaftTypeConfig> StateMachineStoreInner<C> {
    fn next_snapshot_idx(&self) -> u64 {
        self.snapshot_idx.fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// Defines a state machine for the Raft cluster.
///
/// This is a newtype wrapper around `Arc<Mutex<StateMachineStoreInner<C>>>` to satisfy
/// Rust's orphan rules when implementing foreign traits.
#[derive(Debug)]
pub struct StateMachineStore<C: RaftTypeConfig>(Arc<Mutex<StateMachineStoreInner<C>>>);

impl<C: RaftTypeConfig> Default for StateMachineStore<C> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(StateMachineStoreInner::default())))
    }
}

impl<C: RaftTypeConfig> Clone for StateMachineStore<C> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<C: RaftTypeConfig> StateMachineStore<C> {
    pub fn inner(&self) -> &Arc<Mutex<StateMachineStoreInner<C>>> {
        &self.0
    }
}

impl<C> RaftSnapshotBuilder<C> for StateMachineStore<C>
where C: RaftTypeConfig<D = types_kv::Request, R = types_kv::Response, SnapshotData = Cursor<Vec<u8>>, Entry = Entry<C>>
{
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot<C>, io::Error> {
        let mut inner = self.0.lock().await;

        let data =
            serde_json::to_vec(&inner.state_machine.data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let snapshot_idx = inner.next_snapshot_idx();
        let snapshot_id = if let Some(last) = inner.last_applied_log.clone() {
            format!("{}-{}-{}", last.committed_leader_id(), last.index(), snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        };

        let meta = SnapshotMeta {
            last_log_id: inner.last_applied_log.clone(),
            last_membership: inner.last_membership.clone(),
            snapshot_id,
        };

        let snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        inner.current_snapshot = Some(snapshot);

        Ok(Snapshot {
            meta,
            snapshot: Cursor::new(data),
        })
    }
}

impl<C> RaftStateMachine<C> for StateMachineStore<C>
where C: RaftTypeConfig<D = types_kv::Request, R = types_kv::Response, SnapshotData = Cursor<Vec<u8>>, Entry = Entry<C>>
{
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogId<C>>, StoredMembership<C>), io::Error> {
        let inner = self.0.lock().await;
        Ok((inner.last_applied_log.clone(), inner.last_membership.clone()))
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<C>, io::Error>> + Unpin + OptionalSend {
        let mut inner = self.0.lock().await;

        while let Some((entry, responder)) = entries.try_next().await? {
            tracing::debug!(%entry.log_id, "replicate to sm");

            inner.last_applied_log = Some(entry.log_id.clone());

            let response = match &entry.payload {
                EntryPayload::Blank => types_kv::Response::none(),
                EntryPayload::Normal(req) => match req {
                    types_kv::Request::Set { key, value } => {
                        inner.state_machine.data.insert(key.clone(), value.clone());
                        types_kv::Response::new(value.clone())
                    }
                },
                EntryPayload::Membership(mem) => {
                    inner.last_membership = StoredMembership::new(Some(entry.log_id.clone()), mem.clone());
                    types_kv::Response::none()
                }
            };

            if let Some(responder) = responder {
                responder.send(response);
            }
        }
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&mut self) -> Result<C::SnapshotData, io::Error> {
        Ok(Cursor::new(Vec::new()))
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(&mut self, meta: &SnapshotMeta<C>, snapshot: C::SnapshotData) -> Result<(), io::Error> {
        tracing::info!(
            { snapshot_size = snapshot.get_ref().len() },
            "decoding snapshot for installation"
        );

        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        let updated_state_machine_data: BTreeMap<String, String> =
            serde_json::from_slice(&new_snapshot.data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let mut inner = self.0.lock().await;
        inner.last_applied_log = meta.last_log_id.clone();
        inner.last_membership = meta.last_membership.clone();
        inner.state_machine = StateMachineData {
            data: updated_state_machine_data,
        };
        inner.current_snapshot = Some(new_snapshot);

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<C>>, io::Error> {
        let inner = self.0.lock().await;
        match &inner.current_snapshot {
            Some(snapshot) => {
                let data = snapshot.data.clone();
                Ok(Some(Snapshot {
                    meta: snapshot.meta.clone(),
                    snapshot: Cursor::new(data),
                }))
            }
            None => Ok(None),
        }
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}
