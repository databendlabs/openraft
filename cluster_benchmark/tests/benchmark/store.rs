mod test;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use openraft::async_trait::async_trait;
use openraft::storage::LogFlushed;
use openraft::storage::LogState;
use openraft::storage::RaftLogReader;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftSnapshotBuilder;
use openraft::storage::RaftStateMachine;
use openraft::storage::Snapshot;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::RaftLogId;
use openraft::RaftTypeConfig;
use openraft::SnapshotMeta;
use openraft::StorageError;
use openraft::StorageIOError;
use openraft::StoredMembership;
use openraft::TokioRuntime;
use openraft::Vote;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientRequest {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientResponse {}

pub type NodeId = u64;

openraft::declare_raft_types!(
    pub TypeConfig: D = ClientRequest, R = ClientResponse, NodeId = NodeId, Node = (),
    Entry = Entry<TypeConfig>, SnapshotData = Cursor<Vec<u8>>, AsyncRuntime = TokioRuntime
);

#[derive(Debug)]
pub struct StoredSnapshot {
    pub meta: SnapshotMeta<NodeId, ()>,
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct StateMachine {
    pub last_applied_log: Option<LogId<NodeId>>,
    pub last_membership: StoredMembership<NodeId, ()>,
}

pub struct LogStore {
    vote: RwLock<Option<Vote<NodeId>>>,
    log: RwLock<BTreeMap<u64, Entry<TypeConfig>>>,
    last_purged_log_id: RwLock<Option<LogId<NodeId>>>,
}

impl LogStore {
    pub fn new() -> Self {
        let log = RwLock::new(BTreeMap::new());

        Self {
            vote: RwLock::new(None),
            log,
            last_purged_log_id: RwLock::new(None),
        }
    }

    pub async fn new_async() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

impl Default for LogStore {
    fn default() -> Self {
        Self::new()
    }
}

pub struct StateMachineStore {
    sm: RwLock<StateMachine>,
    snapshot_idx: AtomicU64,
    current_snapshot: RwLock<Option<StoredSnapshot>>,
}

impl StateMachineStore {
    pub fn new() -> Self {
        Self {
            sm: RwLock::new(StateMachine::default()),
            snapshot_idx: AtomicU64::new(0),
            current_snapshot: RwLock::new(None),
        }
    }
}

#[async_trait]
impl RaftLogReader<TypeConfig> for Arc<LogStore> {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<NodeId>> {
        let mut entries = vec![];
        {
            let log = self.log.read().await;
            for (_, ent) in log.range(range.clone()) {
                entries.push(ent.clone());
            }
        };

        Ok(entries)
    }
}

#[async_trait]
impl RaftSnapshotBuilder<TypeConfig> for Arc<StateMachineStore> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<NodeId>> {
        let data;
        let last_applied_log;
        let last_membership;

        {
            // Serialize the data of the state machine.
            let sm = self.sm.read().await;
            data = serde_json::to_vec(&*sm).map_err(|e| StorageIOError::read_state_machine(&e))?;

            last_applied_log = sm.last_applied_log;
            last_membership = sm.last_membership.clone();
        }

        let snapshot_size = data.len();

        let snapshot_idx = self.snapshot_idx.fetch_add(1, Ordering::Relaxed);

        let snapshot_id = if let Some(last) = last_applied_log {
            format!("{}-{}-{}", last.leader_id, last.index, snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        };

        let meta = SnapshotMeta {
            last_log_id: last_applied_log,
            last_membership,
            snapshot_id,
        };

        let snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        {
            let mut current_snapshot = self.current_snapshot.write().await;
            *current_snapshot = Some(snapshot);
        }

        tracing::info!(snapshot_size, "log compaction complete");

        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

#[async_trait]
impl RaftLogStorage<TypeConfig> for Arc<LogStore> {
    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError<NodeId>> {
        let log = self.log.read().await;
        let last_serialized = log.iter().rev().next().map(|(_, ent)| ent);

        let last = match last_serialized {
            None => None,
            Some(ent) => Some(*ent.get_log_id()),
        };

        let last_purged = self.last_purged_log_id.read().await.clone();

        let last = match last {
            None => last_purged,
            Some(x) => Some(x),
        };

        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id: last,
        })
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn save_vote(&mut self, vote: &Vote<NodeId>) -> Result<(), StorageError<NodeId>> {
        let mut v = self.vote.write().await;
        *v = Some(*vote);
        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<NodeId>>, StorageError<NodeId>> {
        Ok(self.vote.read().await.clone())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn truncate(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        let mut log = self.log.write().await;
        log.split_off(&log_id.index);

        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn purge(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        {
            let mut p = self.last_purged_log_id.write().await;
            *p = Some(log_id);
        }

        let mut log = self.log.write().await;
        *log = log.split_off(&(log_id.index + 1));

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip_all)]
    async fn append<I>(&mut self, entries: I, callback: LogFlushed<NodeId>) -> Result<(), StorageError<NodeId>>
    where I: IntoIterator<Item = Entry<TypeConfig>> + Send {
        {
            let mut log = self.log.write().await;
            log.extend(entries.into_iter().map(|entry| (entry.get_log_id().index, entry)));
        }
        callback.log_io_completed(Ok(()));
        Ok(())
    }

    type LogReader = Self;

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }
}

#[async_trait]
impl RaftStateMachine<TypeConfig> for Arc<StateMachineStore> {
    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogId<NodeId>>, StoredMembership<NodeId, ()>), StorageError<NodeId>> {
        let sm = self.sm.read().await;
        Ok((sm.last_applied_log, sm.last_membership.clone()))
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<ClientResponse>, StorageError<NodeId>>
    where I: IntoIterator<Item = Entry<TypeConfig>> + Send {
        let mut sm = self.sm.write().await;

        let it = entries.into_iter();
        let mut res = Vec::with_capacity(it.size_hint().1.unwrap_or(0));

        for entry in it {
            sm.last_applied_log = Some(entry.log_id);

            match entry.payload {
                EntryPayload::Blank => res.push(ClientResponse {}),
                EntryPayload::Normal(_) => res.push(ClientResponse {}),
                EntryPayload::Membership(ref mem) => {
                    sm.last_membership = StoredMembership::new(Some(entry.log_id), mem.clone());
                    res.push(ClientResponse {})
                }
            };
        }
        Ok(res)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<<TypeConfig as RaftTypeConfig>::SnapshotData>, StorageError<NodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<NodeId, ()>,
        snapshot: Box<<TypeConfig as RaftTypeConfig>::SnapshotData>,
    ) -> Result<(), StorageError<NodeId>> {
        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        // Update the state machine.
        {
            let new_sm: StateMachine = serde_json::from_slice(&new_snapshot.data)
                .map_err(|e| StorageIOError::read_snapshot(Some(new_snapshot.meta.signature()), &e))?;
            let mut sm = self.sm.write().await;
            *sm = new_sm;
        }

        // Update current snapshot.
        let mut current_snapshot = self.current_snapshot.write().await;
        *current_snapshot = Some(new_snapshot);
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TypeConfig>>, StorageError<NodeId>> {
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

    type SnapshotBuilder = Self;

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}
