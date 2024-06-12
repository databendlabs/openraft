#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]

#[cfg(test)] mod test;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::Mutex;

use openraft::alias::SnapshotDataOf;
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
use openraft::OptionalSend;
use openraft::RaftLogId;
use openraft::SnapshotMeta;
use openraft::StorageError;
use openraft::StorageIOError;
use openraft::StoredMembership;
use openraft::Vote;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;
use tokio::time::Duration;

/// The application data request type which the `MemStore` works with.
///
/// Conceptually, for demo purposes, this represents an update to a client's status info,
/// returning the previously recorded status.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientRequest {
    /// The ID of the client which has sent the request.
    pub client: String,

    /// The serial number of this request.
    pub serial: u64,

    /// A string describing the status of the client. For a real application, this should probably
    /// be an enum representing all of the various types of requests / operations which a client
    /// can perform.
    pub status: String,
}

/// Helper trait to build `ClientRequest` for `MemStore` in generic test code.
pub trait IntoMemClientRequest<T> {
    fn make_request(client_id: impl ToString, serial: u64) -> T;
}

impl IntoMemClientRequest<ClientRequest> for ClientRequest {
    fn make_request(client_id: impl ToString, serial: u64) -> Self {
        Self {
            client: client_id.to_string(),
            serial,
            status: format!("request-{}", serial),
        }
    }
}

/// The application data response type which the `MemStore` works with.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientResponse(pub Option<String>);

pub type MemNodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration for `MemStore`.
    pub TypeConfig:
        D = ClientRequest,
        R = ClientResponse,
        Node = (),
);

/// The application snapshot type which the `MemStore` works with.
#[derive(Debug)]
pub struct MemStoreSnapshot {
    pub meta: SnapshotMeta<TypeConfig>,

    /// The data of the state machine at the time of this snapshot.
    pub data: Vec<u8>,
}

/// The state machine of the `MemStore`.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct MemStoreStateMachine {
    pub last_applied_log: Option<LogId<MemNodeId>>,

    pub last_membership: StoredMembership<TypeConfig>,

    /// A mapping of client IDs to their state info.
    pub client_serial_responses: HashMap<String, (u64, Option<String>)>,
    /// The current status of a client by ID.
    pub client_status: HashMap<String, String>,
}

#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[derive(PartialOrd, Ord)]
pub enum BlockOperation {
    /// Block building a snapshot but does not hold a lock on the state machine.
    /// This will prevent building snapshot returning but should not block applying entries.
    DelayBuildingSnapshot,
    BuildSnapshot,
    PurgeLog,
}

/// Block operations for testing purposes.
#[derive(Clone, Debug, Default)]
pub struct BlockConfig {
    inner: Arc<Mutex<BTreeMap<BlockOperation, Duration>>>,
}

impl BlockConfig {
    /// Block an operation for testing purposes.
    pub fn set_blocking(&self, block: BlockOperation, d: Duration) {
        self.inner.lock().unwrap().insert(block, d);
    }

    /// Get the blocking flag for an operation.
    pub fn get_blocking(&self, block: &BlockOperation) -> Option<Duration> {
        self.inner.lock().unwrap().get(block).cloned()
    }

    /// Clear a blocking flag for an operation.
    pub fn clear_blocking(&mut self, block: BlockOperation) {
        self.inner.lock().unwrap().remove(&block);
    }
}

/// An in-memory log storage implementing the `RaftLogStorage` trait.
pub struct MemLogStore {
    last_purged_log_id: RwLock<Option<LogId<MemNodeId>>>,

    committed: RwLock<Option<LogId<MemNodeId>>>,

    /// The Raft log. Logs are stored in serialized json.
    log: RwLock<BTreeMap<u64, String>>,

    /// Block operations for testing purposes.
    block: BlockConfig,

    /// The current hard state.
    vote: RwLock<Option<Vote<MemNodeId>>>,
}

impl MemLogStore {
    pub fn new(block: BlockConfig) -> Self {
        let log = RwLock::new(BTreeMap::new());

        Self {
            last_purged_log_id: RwLock::new(None),
            committed: RwLock::new(None),
            log,
            block,
            vote: RwLock::new(None),
        }
    }
}

/// An in-memory key-value storage implementing the `RaftStateMachine` trait.
pub struct MemStateMachine {
    /// The Raft state machine.
    sm: RwLock<MemStoreStateMachine>,

    snapshot_idx: Arc<Mutex<u64>>,

    /// The current snapshot.
    current_snapshot: RwLock<Option<MemStoreSnapshot>>,

    /// Block operations for testing purposes.
    pub block: BlockConfig,
}

impl MemStateMachine {
    pub fn new(block: BlockConfig) -> Self {
        let sm = RwLock::new(MemStoreStateMachine::default());
        let current_snapshot = RwLock::new(None);

        Self {
            sm,
            snapshot_idx: Arc::new(Mutex::new(0)),
            current_snapshot,
            block,
        }
    }

    /// Remove the current snapshot.
    ///
    /// This method is only used for testing purposes.
    pub async fn drop_snapshot(&self) {
        let mut current = self.current_snapshot.write().await;
        *current = None;
    }

    /// Get a handle to the state machine for testing purposes.
    pub async fn get_state_machine(&self) -> MemStoreStateMachine {
        self.sm.write().await.clone()
    }

    /// Clear the state machine for testing purposes.
    pub async fn clear_state_machine(&self) {
        let mut sm = self.sm.write().await;
        *sm = MemStoreStateMachine::default();
    }
}

pub fn new_mem_store() -> (Arc<MemLogStore>, Arc<MemStateMachine>) {
    let block = BlockConfig::default();
    (
        Arc::new(MemLogStore::new(block.clone())),
        Arc::new(MemStateMachine::new(block)),
    )
}

impl RaftLogReader<TypeConfig> for Arc<MemLogStore> {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<MemNodeId>> {
        let mut entries = vec![];
        {
            let log = self.log.read().await;
            for (_, serialized) in log.range(range.clone()) {
                let ent = serde_json::from_str(serialized).map_err(|e| StorageIOError::read_logs(&e))?;
                entries.push(ent);
            }
        };

        Ok(entries)
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<MemNodeId>>, StorageError<MemNodeId>> {
        Ok(*self.vote.read().await)
    }
}

impl RaftSnapshotBuilder<TypeConfig> for Arc<MemStateMachine> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<MemNodeId>> {
        let data;
        let last_applied_log;
        let last_membership;

        if let Some(d) = self.block.get_blocking(&BlockOperation::DelayBuildingSnapshot) {
            tracing::info!(?d, "delay snapshot build");
            tokio::time::sleep(d).await;
        }

        {
            // Serialize the data of the state machine.
            let sm = self.sm.read().await;
            data = serde_json::to_vec(&*sm).map_err(|e| StorageIOError::read_state_machine(&e))?;

            last_applied_log = sm.last_applied_log;
            last_membership = sm.last_membership.clone();

            if let Some(d) = self.block.get_blocking(&BlockOperation::BuildSnapshot) {
                tracing::info!(?d, "blocking snapshot build");
                tokio::time::sleep(d).await;
            }
        }

        let snapshot_size = data.len();

        let snapshot_idx = {
            let mut l = self.snapshot_idx.lock().unwrap();
            *l += 1;
            *l
        };

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

        let snapshot = MemStoreSnapshot {
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

impl RaftLogStorage<TypeConfig> for Arc<MemLogStore> {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError<MemNodeId>> {
        let log = self.log.read().await;
        let last_serialized = log.iter().next_back().map(|(_, ent)| ent);

        let last = match last_serialized {
            None => None,
            Some(serialized) => {
                let ent: Entry<TypeConfig> =
                    serde_json::from_str(serialized).map_err(|e| StorageIOError::read_logs(&e))?;
                Some(*ent.get_log_id())
            }
        };

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

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    #[tracing::instrument(level = "trace", skip(self, callback))]
    async fn save_vote(
        &mut self, vote: &Vote<MemNodeId>, callback: LogFlushed<TypeConfig>
    ) -> Result<(), StorageError<MemNodeId>> {
        tracing::debug!(?vote, "save_vote");
        let mut h = self.vote.write().await;
        *h = Some(*vote);
        callback.log_io_completed(Ok(()));
        Ok(())
    }

    async fn save_committed(&mut self, committed: Option<LogId<MemNodeId>>) -> Result<(), StorageError<MemNodeId>> {
        tracing::debug!(?committed, "save_committed");
        let mut c = self.committed.write().await;
        *c = committed;
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogId<MemNodeId>>, StorageError<MemNodeId>> {
        Ok(*self.committed.read().await)
    }

    #[tracing::instrument(level = "trace", skip_all)]
    async fn append<I>(&mut self, entries: I, callback: LogFlushed<TypeConfig>) -> Result<(), StorageError<MemNodeId>>
    where I: IntoIterator<Item = Entry<TypeConfig>> + OptionalSend {
        let mut log = self.log.write().await;
        for entry in entries {
            let s =
                serde_json::to_string(&entry).map_err(|e| StorageIOError::write_log_entry(*entry.get_log_id(), &e))?;
            log.insert(entry.log_id.index, s);
        }

        callback.log_io_completed(Ok(()));
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn truncate(&mut self, log_id: LogId<MemNodeId>) -> Result<(), StorageError<MemNodeId>> {
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

    #[tracing::instrument(level = "debug", skip_all)]
    async fn purge(&mut self, log_id: LogId<MemNodeId>) -> Result<(), StorageError<MemNodeId>> {
        tracing::debug!("purge_log_upto: {:?}", log_id);

        if let Some(d) = self.block.get_blocking(&BlockOperation::PurgeLog) {
            tracing::info!(?d, "block purging log");
            tokio::time::sleep(d).await;
        }

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
}

impl RaftStateMachine<TypeConfig> for Arc<MemStateMachine> {
    type SnapshotBuilder = Self;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogId<MemNodeId>>, StoredMembership<TypeConfig>), StorageError<MemNodeId>> {
        let sm = self.sm.read().await;
        Ok((sm.last_applied_log, sm.last_membership.clone()))
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn apply<I>(&mut self, entries: I) -> Result<Vec<ClientResponse>, StorageError<MemNodeId>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let mut res = Vec::new();

        let mut sm = self.sm.write().await;

        for entry in entries {
            tracing::debug!(%entry.log_id, "replicate to sm");

            sm.last_applied_log = Some(entry.log_id);

            match entry.payload {
                EntryPayload::Blank => res.push(ClientResponse(None)),
                EntryPayload::Normal(ref data) => {
                    if let Some((serial, r)) = sm.client_serial_responses.get(&data.client) {
                        if serial == &data.serial {
                            res.push(ClientResponse(r.clone()));
                            continue;
                        }
                    }
                    let previous = sm.client_status.insert(data.client.clone(), data.status.clone());
                    sm.client_serial_responses.insert(data.client.clone(), (data.serial, previous.clone()));
                    res.push(ClientResponse(previous));
                }
                EntryPayload::Membership(ref mem) => {
                    sm.last_membership = StoredMembership::new(Some(entry.log_id), mem.clone());
                    res.push(ClientResponse(None))
                }
            };
        }
        Ok(res)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&mut self) -> Result<Box<SnapshotDataOf<TypeConfig>>, StorageError<MemNodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<TypeConfig>,
        snapshot: Box<SnapshotDataOf<TypeConfig>>,
    ) -> Result<(), StorageError<MemNodeId>> {
        tracing::info!(
            { snapshot_size = snapshot.get_ref().len() },
            "decoding snapshot for installation"
        );

        let new_snapshot = MemStoreSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        {
            let t = &new_snapshot.data;
            let y = std::str::from_utf8(t).unwrap();
            tracing::debug!("SNAP META:{:?}", meta);
            tracing::debug!("JSON SNAP DATA:{}", y);
        }

        // Update the state machine.
        {
            let new_sm: MemStoreStateMachine = serde_json::from_slice(&new_snapshot.data)
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
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TypeConfig>>, StorageError<MemNodeId>> {
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
