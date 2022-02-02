use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::Mutex;

use anyerror::AnyError;
use openraft::async_trait::async_trait;
use openraft::raft::Entry;
use openraft::raft::EntryPayload;
use openraft::storage::LogState;
use openraft::storage::Snapshot;
use openraft::AppData;
use openraft::AppDataResponse;
use openraft::EffectiveMembership;
use openraft::ErrorSubject;
use openraft::ErrorVerb;
use openraft::LogId;
use openraft::NodeId;
use openraft::RaftStorage;
use openraft::SnapshotMeta;
use openraft::StateMachineChanges;
use openraft::StorageError;
use openraft::StorageIOError;
use openraft::Vote;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

/// Request is a command to modify the state machine.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ExampleRequest {
    Set { key: String, value: String },
    AddNode { id: NodeId, addr: String },
}

impl AppData for ExampleRequest {}

/// The state after applied a request.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExampleResponse {
    pub value: Option<String>,
}

impl AppDataResponse for ExampleResponse {}

#[derive(Debug)]
pub struct ExampleSnapshot {
    pub meta: SnapshotMeta,

    /// The data of the state machine at the time of this snapshot.
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct ExampleStateMachine {
    pub last_applied_log: Option<LogId>,

    pub last_membership: Option<EffectiveMembership>,

    /// Node addresses in this cluster.
    pub nodes: BTreeMap<NodeId, String>,

    /// Application data.
    pub kvs: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
pub struct ExampleStore {
    last_purged_log_id: RwLock<Option<LogId>>,

    /// The Raft log.
    log: RwLock<BTreeMap<u64, Entry<ExampleRequest>>>,

    /// The Raft state machine.
    pub sm: RwLock<ExampleStateMachine>,

    /// The current granted vote.
    vote: RwLock<Option<Vote>>,

    snapshot_idx: Arc<Mutex<u64>>,

    current_snapshot: RwLock<Option<ExampleSnapshot>>,
}

#[async_trait]
impl RaftStorage<ExampleRequest, ExampleResponse> for ExampleStore {
    type SnapshotData = Cursor<Vec<u8>>;

    #[tracing::instrument(level = "trace", skip(self))]
    async fn save_vote(&self, vote: &Vote) -> Result<(), StorageError> {
        let mut h = self.vote.write().await;

        *h = Some(*vote);
        Ok(())
    }

    async fn read_vote(&self) -> Result<Option<Vote>, StorageError> {
        Ok(*self.vote.read().await)
    }

    async fn get_log_state(&self) -> Result<LogState, StorageError> {
        let log = self.log.read().await;
        let last = log.iter().rev().next().map(|(_, ent)| ent.log_id);

        let last_purged = *self.last_purged_log_id.read().await;

        let last = match last {
            None => last_purged,
            Some(x) => Some(x),
        };

        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id: last,
        })
    }

    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &self,
        range: RB,
    ) -> Result<Vec<Entry<ExampleRequest>>, StorageError> {
        let res = {
            let log = self.log.read().await;
            log.range(range.clone()).map(|(_, val)| val.clone()).collect::<Vec<_>>()
        };

        Ok(res)
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn append_to_log(&self, entries: &[&Entry<ExampleRequest>]) -> Result<(), StorageError> {
        let mut log = self.log.write().await;
        for entry in entries {
            log.insert(entry.log_id.index, (*entry).clone());
        }
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn delete_conflict_logs_since(&self, log_id: LogId) -> Result<(), StorageError> {
        tracing::debug!("delete_log: [{:?}, +oo)", log_id);

        {
            let mut log = self.log.write().await;

            let keys = log.range(log_id.index..).map(|(k, _v)| *k).collect::<Vec<_>>();
            for key in keys {
                log.remove(&key);
            }
        }

        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn purge_logs_upto(&self, log_id: LogId) -> Result<(), StorageError> {
        tracing::debug!("delete_log: [{:?}, +oo)", log_id);

        {
            let mut ld = self.last_purged_log_id.write().await;
            assert!(*ld <= Some(log_id));
            *ld = Some(log_id);
        }

        {
            let mut log = self.log.write().await;

            let keys = log.range(..=log_id.index).map(|(k, _v)| *k).collect::<Vec<_>>();
            for key in keys {
                log.remove(&key);
            }
        }

        Ok(())
    }

    async fn last_applied_state(&self) -> Result<(Option<LogId>, Option<EffectiveMembership>), StorageError> {
        let sm = self.sm.read().await;
        Ok((sm.last_applied_log, sm.last_membership.clone()))
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn apply_to_state_machine(
        &self,
        entries: &[&Entry<ExampleRequest>],
    ) -> Result<Vec<ExampleResponse>, StorageError> {
        let mut res = Vec::with_capacity(entries.len());

        let mut sm = self.sm.write().await;

        for entry in entries {
            tracing::debug!(%entry.log_id, "replicate to sm");

            sm.last_applied_log = Some(entry.log_id);

            match entry.payload {
                EntryPayload::Blank => res.push(ExampleResponse { value: None }),
                EntryPayload::Normal(ref req) => match req {
                    ExampleRequest::Set { key, value } => {
                        sm.kvs.insert(key.clone(), value.clone());
                        res.push(ExampleResponse {
                            value: Some(value.clone()),
                        })
                    }
                    ExampleRequest::AddNode { id, addr } => {
                        sm.nodes.insert(*id, addr.clone());
                        res.push(ExampleResponse {
                            value: Some(format!("cluster: {:?}", sm.nodes)),
                        })
                    }
                },
                EntryPayload::Membership(ref mem) => {
                    sm.last_membership = Some(EffectiveMembership {
                        log_id: entry.log_id,
                        membership: mem.clone(),
                    });
                    res.push(ExampleResponse { value: None })
                }
            };
        }
        Ok(res)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&self) -> Result<Snapshot<Self::SnapshotData>, StorageError> {
        let (data, last_applied_log);

        {
            // Serialize the data of the state machine.
            let sm = self.sm.read().await;
            data = serde_json::to_vec(&*sm)
                .map_err(|e| StorageIOError::new(ErrorSubject::StateMachine, ErrorVerb::Read, AnyError::new(&e)))?;

            last_applied_log = sm.last_applied_log;
        }

        let last_applied_log = match last_applied_log {
            None => {
                panic!("can not compact empty state machine");
            }
            Some(x) => x,
        };

        let snapshot_idx = {
            let mut l = self.snapshot_idx.lock().unwrap();
            *l += 1;
            *l
        };

        let snapshot_id = format!(
            "{}-{}-{}",
            last_applied_log.leader_id, last_applied_log.index, snapshot_idx
        );

        let meta = SnapshotMeta {
            last_log_id: last_applied_log,
            snapshot_id,
        };

        let snapshot = ExampleSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        {
            let mut current_snapshot = self.current_snapshot.write().await;
            *current_snapshot = Some(snapshot);
        }

        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&self) -> Result<Box<Self::SnapshotData>, StorageError> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(
        &self,
        meta: &SnapshotMeta,
        snapshot: Box<Self::SnapshotData>,
    ) -> Result<StateMachineChanges, StorageError> {
        tracing::info!(
            { snapshot_size = snapshot.get_ref().len() },
            "decoding snapshot for installation"
        );

        let new_snapshot = ExampleSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        // Update the state machine.
        {
            let new_sm: ExampleStateMachine = serde_json::from_slice(&new_snapshot.data).map_err(|e| {
                StorageIOError::new(
                    ErrorSubject::Snapshot(new_snapshot.meta.clone()),
                    ErrorVerb::Read,
                    AnyError::new(&e),
                )
            })?;
            let mut sm = self.sm.write().await;
            *sm = new_sm;
        }

        // Update current snapshot.
        let mut current_snapshot = self.current_snapshot.write().await;
        *current_snapshot = Some(new_snapshot);
        Ok(StateMachineChanges {
            last_applied: meta.last_log_id,
            is_snapshot: true,
        })
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&self) -> Result<Option<Snapshot<Self::SnapshotData>>, StorageError> {
        match &*self.current_snapshot.read().await {
            Some(snapshot) => {
                let data = snapshot.data.clone();
                Ok(Some(Snapshot {
                    meta: snapshot.meta.clone(),
                    snapshot: Box::new(Cursor::new(data)),
                }))
            }
            None => Ok(None),
        }
    }
}
