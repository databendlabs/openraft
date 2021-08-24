#![doc = include_str!("../README.md")]

#[cfg(test)]
mod test;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Result;
use async_raft::async_trait::async_trait;
use async_raft::raft::Entry;
use async_raft::raft::EntryPayload;
use async_raft::raft::MembershipConfig;
use async_raft::storage::HardState;
use async_raft::storage::InitialState;
use async_raft::storage::Snapshot;
use async_raft::AppData;
use async_raft::AppDataResponse;
use async_raft::LogId;
use async_raft::NodeId;
use async_raft::RaftStorage;
use async_raft::RaftStorageDebug;
use async_raft::SnapshotMeta;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;
use tokio::sync::RwLock;

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

impl AppData for ClientRequest {}

/// The application data response type which the `MemStore` works with.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientResponse(Option<String>);

impl AppDataResponse for ClientResponse {}

/// Error used to trigger Raft shutdown from storage.
#[derive(Clone, Debug, Error)]
pub enum ShutdownError {
    #[error("unsafe storage error")]
    UnsafeStorageError,
}

/// The application snapshot type which the `MemStore` works with.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MemStoreSnapshot {
    pub meta: SnapshotMeta,

    /// The data of the state machine at the time of this snapshot.
    pub data: Vec<u8>,
}

/// The state machine of the `MemStore`.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct MemStoreStateMachine {
    pub last_applied_log: LogId,

    pub last_membership: Option<MembershipConfig>,

    /// A mapping of client IDs to their state info.
    pub client_serial_responses: HashMap<String, (u64, Option<String>)>,
    /// The current status of a client by ID.
    pub client_status: HashMap<String, String>,
}

/// An in-memory storage system implementing the `async_raft::RaftStorage` trait.
pub struct MemStore {
    /// The ID of the Raft node for which this memory storage instances is configured.
    id: NodeId,
    /// The Raft log.
    log: RwLock<BTreeMap<u64, Entry<ClientRequest>>>,
    /// The Raft state machine.
    sm: RwLock<MemStoreStateMachine>,
    /// The current hard state.
    hs: RwLock<Option<HardState>>,

    snapshot_idx: Arc<Mutex<u64>>,
    /// The current snapshot.
    current_snapshot: RwLock<Option<MemStoreSnapshot>>,
}

impl MemStore {
    /// Create a new `MemStore` instance.
    pub fn new(id: NodeId) -> Self {
        let log = RwLock::new(BTreeMap::new());
        let sm = RwLock::new(MemStoreStateMachine::default());
        let hs = RwLock::new(None);
        let current_snapshot = RwLock::new(None);
        Self {
            id,
            log,
            sm,
            hs,
            snapshot_idx: Arc::new(Mutex::new(0)),
            current_snapshot,
        }
    }

    /// Create a new `MemStore` instance with some existing state (for testing).
    #[cfg(test)]
    pub fn new_with_state(
        id: NodeId,
        log: BTreeMap<u64, Entry<ClientRequest>>,
        sm: MemStoreStateMachine,
        hs: Option<HardState>,
        current_snapshot: Option<MemStoreSnapshot>,
    ) -> Self {
        let log = RwLock::new(log);
        let sm = RwLock::new(sm);
        let hs = RwLock::new(hs);
        let current_snapshot = RwLock::new(current_snapshot);
        Self {
            id,
            log,
            sm,
            hs,
            snapshot_idx: Arc::new(Mutex::new(0)),
            current_snapshot,
        }
    }
}

#[async_trait]
impl RaftStorageDebug<MemStoreStateMachine> for MemStore {
    /// Get a handle to the state machine for testing purposes.
    async fn get_state_machine(&self) -> MemStoreStateMachine {
        self.sm.write().await.clone()
    }

    /// Get a handle to the current hard state for testing purposes.
    async fn read_hard_state(&self) -> Option<HardState> {
        self.hs.read().await.clone()
    }
}

impl MemStore {
    fn find_first_membership_log<'a, T, D>(mut it: T) -> Option<MembershipConfig>
    where
        T: 'a + Iterator<Item = &'a Entry<D>>,
        D: AppData,
    {
        it.find_map(|entry| match &entry.payload {
            EntryPayload::ConfigChange(cfg) => Some(cfg.membership.clone()),
            _ => None,
        })
    }

    /// Go backwards through the log to find the most recent membership config <= `upto_index`.
    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn get_membership_from_log(&self, upto_index: Option<u64>) -> Result<MembershipConfig> {
        let membership = {
            let log = self.log.read().await;

            let reversed_logs = log.values().rev();
            match upto_index {
                Some(upto) => {
                    let skipped = reversed_logs.skip_while(|entry| entry.log_id.index > upto);
                    Self::find_first_membership_log(skipped)
                }
                None => Self::find_first_membership_log(reversed_logs),
            }
        };

        // Find membership stored in state machine.

        let membership = match membership {
            None => self.sm.read().await.last_membership.clone(),
            Some(x) => Some(x),
        };

        // Otherwise, create a default one.

        Ok(match membership {
            Some(cfg) => cfg,
            None => MembershipConfig::new_initial(self.id),
        })
    }
}

#[async_trait]
impl RaftStorage<ClientRequest, ClientResponse> for MemStore {
    type SnapshotData = Cursor<Vec<u8>>;
    type ShutdownError = ShutdownError;

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_membership_config(&self) -> Result<MembershipConfig> {
        self.get_membership_from_log(None).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_initial_state(&self) -> Result<InitialState> {
        let membership = self.get_membership_config().await?;
        let mut hs = self.hs.write().await;
        let log = self.log.read().await;
        let sm = self.sm.read().await;
        match &mut *hs {
            Some(inner) => {
                let last_log_id = match log.values().rev().next() {
                    Some(log) => log.log_id,
                    None => (0, 0).into(),
                };
                let last_applied_log = sm.last_applied_log;
                Ok(InitialState {
                    last_log_id,
                    last_applied_log,
                    hard_state: inner.clone(),
                    membership,
                })
            }
            None => {
                let new = InitialState::new_initial(self.id);
                *hs = Some(new.hard_state.clone());
                Ok(new)
            }
        }
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn save_hard_state(&self, hs: &HardState) -> Result<()> {
        *self.hs.write().await = Some(hs.clone());
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_log_entries(&self, start: u64, stop: u64) -> Result<Vec<Entry<ClientRequest>>> {
        // Invalid request, return empty vec.
        if start > stop {
            tracing::error!("get_log_entries: invalid request, start({}) > stop({})", start, stop);
            return Ok(vec![]);
        }
        let log = self.log.read().await;
        Ok(log.range(start..stop).map(|(_, val)| val.clone()).collect())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn delete_logs_from(&self, start: u64, stop: Option<u64>) -> Result<()> {
        // TODO(xp): never delete the last applied log
        if stop.as_ref().map(|stop| &start > stop).unwrap_or(false) {
            tracing::error!("delete_logs_from: invalid request, start({}) > stop({:?})", start, stop);
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

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn append_to_log(&self, entries: &[&Entry<ClientRequest>]) -> Result<()> {
        let mut log = self.log.write().await;
        for entry in entries {
            log.insert(entry.log_id.index, (*entry).clone());
        }
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn replicate_to_state_machine(&self, entries: &[&Entry<ClientRequest>]) -> Result<Vec<ClientResponse>> {
        let mut sm = self.sm.write().await;
        let mut res = Vec::with_capacity(entries.len());

        for entry in entries {
            tracing::debug!("id:{} replicate to sm index:{}", self.id, entry.log_id.index);

            // TODO(xp) return error if there is out of order apply
            assert_eq!(sm.last_applied_log.index + 1, entry.log_id.index);

            sm.last_applied_log = entry.log_id;

            match entry.payload {
                EntryPayload::Blank => res.push(ClientResponse(None)),
                EntryPayload::PurgedMarker => res.push(ClientResponse(None)),
                EntryPayload::Normal(ref norm) => {
                    let data = &norm.data;
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
                EntryPayload::ConfigChange(ref mem) => {
                    sm.last_membership = Some(mem.membership.clone());
                    res.push(ClientResponse(None))
                }
            };
        }
        Ok(res)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn do_log_compaction(&self) -> Result<Snapshot<Self::SnapshotData>> {
        let (data, last_applied_log);
        let membership_config;
        {
            // Serialize the data of the state machine.
            let sm = self.sm.read().await;
            data = serde_json::to_vec(&*sm)?;
            last_applied_log = sm.last_applied_log;
            membership_config = sm.last_membership.clone().unwrap_or_else(|| MembershipConfig::new_initial(self.id));
        } // Release state machine read lock.

        let snapshot_size = data.len();

        let snapshot_idx = {
            let mut l = self.snapshot_idx.lock().unwrap();
            *l += 1;
            *l
        };

        let meta;
        {
            let mut log = self.log.write().await;
            let mut current_snapshot = self.current_snapshot.write().await;

            // Leaves at least one log or replication can not find out the mismatched log.
            *log = log.split_off(&last_applied_log.index);

            let snapshot_id = format!("{}-{}-{}", last_applied_log.term, last_applied_log.index, snapshot_idx);

            meta = SnapshotMeta {
                last_log_id: last_applied_log,
                snapshot_id,
                membership: membership_config.clone(),
            };

            let snapshot = MemStoreSnapshot {
                meta: meta.clone(),
                data: data.clone(),
            };

            *current_snapshot = Some(snapshot);
        } // Release log & snapshot write locks.

        tracing::info!({ snapshot_size = snapshot_size }, "log compaction complete");
        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&self) -> Result<Box<Self::SnapshotData>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn finalize_snapshot_installation(
        &self,
        meta: &SnapshotMeta,
        snapshot: Box<Self::SnapshotData>,
    ) -> Result<()> {
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

        // Update log.
        {
            let mut log = self.log.write().await;

            // Remove logs that are included in the snapshot.
            // Leave at least one log or the replication can not find out the mismatched log.
            *log = log.split_off(&meta.last_log_id.index);

            // In case there are no log at all, a marker log need to be added to indicate the last log.
            log.insert(meta.last_log_id.index, Entry::new_purged_marker(meta.last_log_id));
        }

        // Update the state machine.
        {
            let new_sm: MemStoreStateMachine = serde_json::from_slice(&new_snapshot.data)?;
            let mut sm = self.sm.write().await;
            *sm = new_sm;
        }

        // Update current snapshot.
        let mut current_snapshot = self.current_snapshot.write().await;
        *current_snapshot = Some(new_snapshot);
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&self) -> Result<Option<Snapshot<Self::SnapshotData>>> {
        match &*self.current_snapshot.read().await {
            Some(snapshot) => {
                // TODO(xp): try not to clone the entire data.
                //           If snapshot.data is Arc<T> that impl AsyncRead etc then the sharing can be done.
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
