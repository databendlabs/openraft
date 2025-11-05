use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Debug;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;

use futures::Stream;
use futures::TryStreamExt;
use opendal::Operator;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;
use serde::Deserialize;
use serde::Serialize;

use crate::decode_buffer;
use crate::encode;
use crate::typ::*;
use crate::TypeConfig;

pub type LogStore = mem_log::LogStore<TypeConfig>;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Request {
    Set { key: String, value: String },
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Request::Set { key, value } => write!(f, "Set {{ key: {}, value: {} }}", key, value),
        }
    }
}

impl Request {
    pub fn set(key: impl ToString, value: impl ToString) -> Self {
        Self::Set {
            key: key.to_string(),
            value: value.to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Response {
    pub value: Option<String>,
}

#[derive(Debug)]
pub struct StoredSnapshot {
    pub meta: SnapshotMeta,

    /// The data of the state machine at the time of this snapshot.
    pub data: SnapshotData,
}

/// Data contained in the Raft state machine.
///
/// Note that we are using `serde` to serialize the
/// `data`, which has a implementation to be serialized. Note that for this test we set both the key
/// and value as String, but you could set any type of value that has the serialization impl.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct StateMachineData {
    pub last_applied: Option<LogId>,

    pub last_membership: StoredMembership,

    /// Application data.
    pub data: BTreeMap<String, String>,
}

/// Defines a state machine for the Raft cluster. This state machine represents a copy of the
/// data for this node. Additionally, it is responsible for storing the last snapshot of the data.
#[derive(Debug)]
pub struct StateMachineStore {
    /// The Raft state machine.
    pub state_machine: tokio::sync::Mutex<StateMachineData>,

    snapshot_idx: Mutex<u64>,
    storage: Operator,

    /// The last received snapshot.
    current_snapshot: Mutex<Option<StoredSnapshot>>,
}

impl StateMachineStore {
    pub fn new(storage: Operator) -> Self {
        Self {
            state_machine: tokio::sync::Mutex::new(StateMachineData::default()),
            snapshot_idx: Mutex::new(0),
            storage,
            current_snapshot: Mutex::new(None),
        }
    }
}

impl RaftSnapshotBuilder<TypeConfig> for Arc<StateMachineStore> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot, io::Error> {
        let data;
        let last_applied_log;
        let last_membership;

        {
            // Serialize the data of the state machine.
            let state_machine = self.state_machine.lock().await.clone();

            last_applied_log = state_machine.last_applied;
            last_membership = state_machine.last_membership.clone();
            data = state_machine;
        }

        let snapshot_idx = {
            let mut l = self.snapshot_idx.lock().unwrap();
            *l += 1;
            *l
        };

        let snapshot_id = if let Some(last) = last_applied_log {
            format!("{}-{}-{}", last.committed_leader_id(), last.index(), snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        };

        // Save the snapshot to the storage.
        //
        // In this example, we use `snapshot_id` as the snapshot store path.
        // Users can design their own logic for this like using uuid.
        self.storage.write(&snapshot_id, encode(&data)).await.unwrap();

        let meta = SnapshotMeta {
            last_log_id: last_applied_log,
            last_membership,
            snapshot_id: snapshot_id.clone(),
        };

        let snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot_id.clone(),
        };

        {
            let mut current_snapshot = self.current_snapshot.lock().unwrap();
            *current_snapshot = Some(snapshot);
        }

        Ok(Snapshot {
            meta,
            snapshot: snapshot_id,
        })
    }
}

impl RaftStateMachine<TypeConfig> for Arc<StateMachineStore> {
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogId>, StoredMembership), io::Error> {
        let state_machine = self.state_machine.lock().await;
        Ok((state_machine.last_applied, state_machine.last_membership.clone()))
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend {
        let mut sm = self.state_machine.lock().await;

        while let Some((entry, responder)) = entries.try_next().await? {
            tracing::debug!(%entry.log_id, "replicate to sm");

            sm.last_applied = Some(entry.log_id);

            let response = match entry.payload {
                EntryPayload::Blank => Response { value: None },
                EntryPayload::Normal(ref req) => match req {
                    Request::Set { key, value, .. } => {
                        sm.data.insert(key.clone(), value.clone());
                        Response {
                            value: Some(value.clone()),
                        }
                    }
                },
                EntryPayload::Membership(ref mem) => {
                    sm.last_membership = StoredMembership::new(Some(entry.log_id), mem.clone());
                    Response { value: None }
                }
            };

            if let Some(responder) = responder {
                responder.send(response);
            }
        }
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&mut self) -> Result<SnapshotData, io::Error> {
        Ok(Default::default())
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(&mut self, meta: &SnapshotMeta, snapshot: SnapshotData) -> Result<(), io::Error> {
        tracing::info!("install snapshot");

        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot,
        };

        // Update the state machine.
        {
            let bs = self.storage.read(&new_snapshot.data).await.unwrap();
            let updated_state_machine: StateMachineData = decode_buffer(bs);
            let mut state_machine = self.state_machine.lock().await;
            *state_machine = updated_state_machine;
        }

        // Update current snapshot.
        let mut current_snapshot = self.current_snapshot.lock().unwrap();
        *current_snapshot = Some(new_snapshot);
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot>, io::Error> {
        match &*self.current_snapshot.lock().unwrap() {
            Some(snapshot) => {
                let data = snapshot.data.clone();
                Ok(Some(Snapshot {
                    meta: snapshot.meta.clone(),
                    snapshot: data,
                }))
            }
            None => Ok(None),
        }
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}
