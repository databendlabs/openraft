use std::collections::BTreeMap;
use std::io::Cursor;

use async_raft::async_trait;
use async_raft::{AppData, AppDataResponse, AppError, NodeId, RaftStorage};
use async_raft::raft::{Entry, EntryPayload, EntrySnapshotPointer, MembershipConfig};
use async_raft::storage::{CurrentSnapshotData, HardState, InitialState};
use serde::{Serialize, Deserialize};
use thiserror::Error;
use tokio::sync::RwLock;

/// The application data type which the `MemStore` works with.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MemStoreData {
    /// A simple string payload.
    pub data: String,
}

impl AppData for MemStoreData {}

/// The application data response type which the `MemStore` works with.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MemStoreDataResponse {
    /// The Raft index of the written data payload.
    pub index: u64,
}

impl AppDataResponse for MemStoreDataResponse {}

/// The application error which the `MemStore` works with.
#[derive(Serialize, Deserialize, Debug, Error)]
pub enum MemStoreError {
    /// An application specific error indicating that the given payload is not allowed.
    #[error("the given payload is not allowed")]
    NotAllowed,
    /// An error related to CBOR usage with the snapshot system.
    #[error("{0}")]
    SnapshotCborError(String),
    /// A query was received which was expecting data to be in place which does not exist in the log.
    #[error("a query was received which was expecting data to be in place which does not exist in the log")]
    InconsistentLog,
}

impl AppError for MemStoreError {}

/// The application snapshot type which the `MemStore` works with.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MemStoreSnapshot {
    /// The last index covered by this snapshot.
    pub index: u64,
    /// The term of the last index covered by this snapshot.
    pub term: u64,
    /// The last memberhsip config included in this snapshot.
    pub membership: MembershipConfig,
    /// The data of the state machine at the time of this snapshot.
    pub data: Vec<u8>,
}

/// An in-memory storage system for demo and testing purposes related to `async-raft`.
pub struct MemStore {
    /// The ID of the Raft node for which this memory storage instances is configured.
    id: NodeId,
    /// The Raft log.
    log: RwLock<BTreeMap<u64, Entry<MemStoreData>>>,
    /// The Raft state machine.
    sm: RwLock<BTreeMap<u64, Entry<MemStoreData>>>,
    /// The current hard state.
    hs: RwLock<Option<HardState>>,
    /// The current snapshot.
    current_snapshot: RwLock<Option<MemStoreSnapshot>>,
}

impl MemStore {
    pub fn new(id: NodeId) -> Self {
        let log = RwLock::new(BTreeMap::new());
        let sm = RwLock::new(BTreeMap::new());
        let hs = RwLock::new(None);
        let current_snapshot = RwLock::new(None);
        Self{id, log, sm, hs, current_snapshot}
    }
}

#[async_trait]
impl RaftStorage<MemStoreData, MemStoreDataResponse, MemStoreError> for MemStore {
    type Snapshot = Cursor<Vec<u8>>;

    #[tracing::instrument(level="trace", skip(self))]
    async fn get_initial_state(&self) -> Result<InitialState, MemStoreError> {
        let mut hs = self.hs.write().await;
        let log = self.log.read().await;
        let sm = self.log.read().await;
        match &mut *hs {
            Some(inner) => {
                let (last_log_index, last_log_term) = match log.values().rev().nth(0) {
                    Some(log) => (log.index, log.term),
                    None => (0, 0),
                };
                let last_applied_log = match sm.values().rev().nth(0) {
                    Some(entry) => entry.index,
                    None => 0,
                };
                return Ok(InitialState{
                    last_log_index,
                    last_log_term,
                    last_applied_log,
                    hard_state: inner.clone(),
                });
            }
            None => {
                let new = InitialState::new_initial(self.id);
                *hs = Some(new.hard_state.clone());
                return Ok(new);
            }
        }
    }

    #[tracing::instrument(level="trace", skip(self, hs))]
    async fn save_hard_state(&self, hs: &HardState) -> Result<(), MemStoreError> {
        Ok(*self.hs.write().await = Some(hs.clone()))
    }

    #[tracing::instrument(level="trace", skip(self))]
    async fn get_log_entries(&self, start: u64, stop: u64) -> Result<Vec<Entry<MemStoreData>>, MemStoreError> {
        // Invalid request, return empty vec.
        if &start > &stop {
            tracing::error!("invalid request, start > stop");
            return Ok(vec![]);
        }
        let log = self.log.read().await;
        Ok(log.range(start..stop).map(|(_, val)| val.clone()).collect())
    }

    #[tracing::instrument(level="trace", skip(self))]
    async fn delete_logs_from(&self, start: u64, stop: Option<u64>) -> Result<(), MemStoreError> {
        if stop.as_ref().map(|stop| &start > stop).unwrap_or(false) {
            tracing::error!("invalid request, start > stop");
            return Ok(());
        }
        let mut log = self.log.write().await;

        // If a stop point was specified, delete from start until the given stop point.
        if let Some(stop) = stop.as_ref() {
            for key in start..*stop {
                log.remove(&key);
            }
            return Ok(());
        }
        // Else, just split off the remainder.
        log.split_off(&start);
        Ok(())
    }

    #[tracing::instrument(level="trace", skip(self, entry))]
    async fn append_entry_to_log(&self, entry: &Entry<MemStoreData>) -> Result<(), MemStoreError> {
        let mut log = self.log.write().await;
        log.insert(entry.index, entry.clone());
        Ok(())
    }

    #[tracing::instrument(level="trace", skip(self, entries))]
    async fn replicate_to_log(&self, entries: &[Entry<MemStoreData>]) -> Result<(), MemStoreError> {
        let mut log = self.log.write().await;
        for entry in entries {
            log.insert(entry.index, entry.clone());
        }
        Ok(())
    }

    #[tracing::instrument(level="trace", skip(self, entry))]
    async fn apply_entry_to_state_machine(&self, entry: &Entry<MemStoreData>) -> Result<MemStoreDataResponse, MemStoreError> {
        let mut sm = self.sm.write().await;
        sm.insert(entry.index, entry.clone());
        Ok(MemStoreDataResponse{index: entry.index})
    }

    #[tracing::instrument(level="trace", skip(self, entries))]
    async fn replicate_to_state_machine(&self, entries: &[Entry<MemStoreData>]) -> Result<(), MemStoreError> {
        let mut sm = self.sm.write().await;
        for entry in entries {
            sm.insert(entry.index, entry.clone());
        }
        Ok(())
    }

    #[tracing::instrument(level="trace", skip(self))]
    async fn do_log_compaction(&self, through: u64) -> Result<CurrentSnapshotData<Self::Snapshot>, MemStoreError> {
        let data;
        {
            // Serialize the data of the state machine.
            let sm = self.sm.read().await;
            data = serde_cbor::to_vec(&*sm).map_err(|err| MemStoreError::SnapshotCborError(err.to_string()))?;
        } // Release state machine read lock.

        let membership_config;
        {
            // Go backwards through the log to find the most recent membership config <= the `through` index.
            let log = self.log.read().await;
            membership_config = log.values().rev()
                .skip_while(|entry| &entry.index > &through)
                .find_map(|entry| match &entry.payload {
                    EntryPayload::ConfigChange(cfg) => Some(cfg.membership.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| MembershipConfig::new_initial(self.id));
        } // Release log read lock.

        let snapshot;
        let term;
        {
            let mut log = self.log.write().await;
            let mut current_snapshot = self.current_snapshot.write().await;
            term = log.get(&through).map(|entry| entry.term).ok_or_else(|| MemStoreError::InconsistentLog)?;
            *log = log.split_off(&through);
            log.insert(through, Entry::new_snapshot_pointer(EntrySnapshotPointer{id: "".into()}, through, term));

            snapshot = MemStoreSnapshot{index: through, term, membership: membership_config.clone(), data};
            *current_snapshot = Some(snapshot.clone());
        } // Release log & snapshot write locks.

        let snapshot_bytes = serde_cbor::to_vec(&snapshot).map_err(|err| MemStoreError::SnapshotCborError(err.to_string()))?;
        Ok(CurrentSnapshotData{
            term, index: through, membership: membership_config.clone(),
            snapshot: Box::new(Cursor::new(snapshot_bytes)),
        })
    }

    #[tracing::instrument(level="trace", skip(self))]
    async fn create_snapshot(&self) -> Result<(String, Box<Self::Snapshot>), MemStoreError> {
        Ok((String::from(""), Box::new(Cursor::new(Vec::new())))) // Snapshot IDs are insignificant to this storage engine.
    }

    #[tracing::instrument(level="trace", skip(self))]
    async fn finalize_snapshot_installation(&self, index: u64, term: u64, delete_through: Option<u64>, id: String, snapshot: Box<Self::Snapshot>) -> Result<(), MemStoreError> {
        let new_snapshot: MemStoreSnapshot = serde_cbor::from_slice(snapshot.get_ref())
            .map_err(|err| MemStoreError::SnapshotCborError(err.to_string()))?;
        // Update log.
        {
            let mut log = self.log.write().await;
            match &delete_through {
                Some(through) => {
                    *log = log.split_off(&(through + 1));
                }
                None => log.clear(),
            }
            log.insert(index, Entry::new_snapshot_pointer(EntrySnapshotPointer{id}, index, term));
        }

        // Update current snapshot.
        let mut current_snapshot = self.current_snapshot.write().await;
        *current_snapshot = Some(new_snapshot);
        Ok(())
    }

    #[tracing::instrument(level="trace", skip(self))]
    async fn get_current_snapshot(&self) -> Result<Option<CurrentSnapshotData<Self::Snapshot>>, MemStoreError> {
        match &*self.current_snapshot.read().await {
            Some(snapshot) => {
                let reader = serde_cbor::to_vec(&snapshot).map_err(|err| MemStoreError::SnapshotCborError(err.to_string()))?;
                Ok(Some(CurrentSnapshotData{
                    index: snapshot.index,
                    term: snapshot.term,
                    membership: snapshot.membership.clone(),
                    snapshot: Box::new(Cursor::new(reader)),
                }))
            }
            None => Ok(None),
        }
    }
}
