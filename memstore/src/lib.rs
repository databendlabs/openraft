#![doc = include_str!("../README.md")]
#![feature(backtrace)]
#![feature(bound_cloned)]

#[cfg(test)]
mod test;

use std::cmp::max;
use std::collections::BTreeMap;
use std::collections::Bound;
use std::collections::HashMap;
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::Mutex;

use async_raft::async_trait::async_trait;
use async_raft::raft::Entry;
use async_raft::raft::EntryPayload;
use async_raft::raft::MembershipConfig;
use async_raft::storage::HardState;
use async_raft::storage::InitialState;
use async_raft::storage::Snapshot;
use async_raft::AppData;
use async_raft::AppDataResponse;
use async_raft::DefensiveError;
use async_raft::ErrorSubject;
use async_raft::ErrorVerb;
use async_raft::LogId;
use async_raft::NodeId;
use async_raft::RaftStorage;
use async_raft::RaftStorageDebug;
use async_raft::SnapshotMeta;
use async_raft::StateMachineChanges;
use async_raft::StorageError;
use async_raft::StorageIOError;
use async_raft::Violation;
use serde::Deserialize;
use serde::Serialize;
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

    pub last_membership: Option<(LogId, MembershipConfig)>,

    /// A mapping of client IDs to their state info.
    pub client_serial_responses: HashMap<String, (u64, Option<String>)>,
    /// The current status of a client by ID.
    pub client_status: HashMap<String, String>,
}

/// An in-memory storage system implementing the `async_raft::RaftStorage` trait.
pub struct MemStore {
    /// Turn on defensive check for inputs.
    defensive: RwLock<bool>,

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
            defensive: RwLock::new(true),
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
            defensive: RwLock::new(true),
            id,
            log,
            sm,
            hs,
            snapshot_idx: Arc::new(Mutex::new(0)),
            current_snapshot,
        }
    }
}

// TODO(xp): elaborate errors
impl MemStore {
    /// Ensure that logs that have greater index than last_applied should have greater log_id.
    /// Invariant must hold: `log.log_id.index > last_applied.index` implies `log.log_id > last_applied`.
    pub async fn defensive_no_dirty_log(&self) -> Result<(), DefensiveError> {
        if !*self.defensive.read().await {
            return Ok(());
        }

        let log = self.log.read().await;
        let sm = self.sm.read().await;
        let last_log_id = log.iter().last().map(|(_index, entry)| entry.log_id).unwrap_or_default();
        let last_applied = sm.last_applied_log;

        if last_log_id.index > last_applied.index && last_log_id < last_applied {
            return Err(DefensiveError::new(
                ErrorSubject::Log(last_log_id),
                Violation::DirtyLog {
                    higher_index_log_id: last_log_id,
                    lower_index_log_id: last_applied,
                },
            ));
        }

        Ok(())
    }

    /// Ensure that current_term must increment for every update, and for every term there could be only one value for
    /// voted_for.
    pub async fn defensive_incremental_hard_state(&self, hs: &HardState) -> Result<(), DefensiveError> {
        if !*self.defensive.read().await {
            return Ok(());
        }

        let h = self.hs.write().await;
        let curr = h.clone().unwrap_or_default();
        if hs.current_term < curr.current_term {
            return Err(DefensiveError::new(
                ErrorSubject::HardState,
                Violation::TermNotAscending {
                    curr: curr.current_term,
                    to: hs.current_term,
                },
            ));
        }

        if hs.current_term == curr.current_term && curr.voted_for.is_some() && hs.voted_for != curr.voted_for {
            return Err(DefensiveError::new(
                ErrorSubject::HardState,
                Violation::VotedForChanged { curr, to: hs.clone() },
            ));
        }

        Ok(())
    }

    pub async fn defensive_consecutive_input<D: AppData>(&self, entries: &[&Entry<D>]) -> Result<(), DefensiveError> {
        if !*self.defensive.read().await {
            return Ok(());
        }

        if entries.is_empty() {
            return Ok(());
        }

        let mut prev_log_id = entries[0].log_id;

        for e in entries.iter().skip(1) {
            if e.log_id.index != prev_log_id.index + 1 {
                return Err(DefensiveError::new(ErrorSubject::Logs, Violation::LogsNonConsecutive {
                    prev: prev_log_id,
                    next: e.log_id,
                }));
            }

            prev_log_id = e.log_id;
        }

        Ok(())
    }

    pub async fn defensive_nonempty_input<D: AppData>(&self, entries: &[&Entry<D>]) -> Result<(), DefensiveError> {
        if !*self.defensive.read().await {
            return Ok(());
        }

        if entries.is_empty() {
            return Err(DefensiveError::new(ErrorSubject::Logs, Violation::LogsEmpty));
        }

        Ok(())
    }

    pub async fn defensive_append_log_index_is_last_plus_one<D: AppData>(
        &self,
        entries: &[&Entry<D>],
    ) -> Result<(), DefensiveError> {
        if !*self.defensive.read().await {
            return Ok(());
        }

        let last_id = self.last_log_id().await;

        let first_id = entries[0].log_id;
        if last_id.index + 1 != first_id.index {
            return Err(DefensiveError::new(
                ErrorSubject::Log(first_id),
                Violation::LogsNonConsecutive {
                    prev: last_id,
                    next: first_id,
                },
            ));
        }

        Ok(())
    }

    pub async fn defensive_append_log_id_gt_last<D: AppData>(
        &self,
        entries: &[&Entry<D>],
    ) -> Result<(), DefensiveError> {
        if !*self.defensive.read().await {
            return Ok(());
        }

        let last_id = self.last_log_id().await;

        let first_id = entries[0].log_id;
        if first_id < last_id {
            return Err(DefensiveError::new(
                ErrorSubject::Log(first_id),
                Violation::LogsNonConsecutive {
                    prev: last_id,
                    next: first_id,
                },
            ));
        }

        Ok(())
    }

    /// Find the last known log id from log or state machine
    /// If no log id found, the default one `0,0` is returned.
    pub async fn last_log_id(&self) -> LogId {
        let log_last_id = {
            let log_last = self.log.read().await;
            log_last.iter().last().map(|(_k, v)| v.log_id).unwrap_or_default()
        };

        let sm_last_id = self.sm.read().await.last_applied_log;

        std::cmp::max(log_last_id, sm_last_id)
    }

    pub async fn defensive_apply_index_is_last_applied_plus_one<D: AppData>(
        &self,
        entries: &[&Entry<D>],
    ) -> Result<(), DefensiveError> {
        if !*self.defensive.read().await {
            return Ok(());
        }

        let last_id = self.sm.read().await.last_applied_log;

        let first_id = entries[0].log_id;
        if last_id.index + 1 != first_id.index {
            return Err(DefensiveError::new(
                ErrorSubject::Apply(first_id),
                Violation::ApplyNonConsecutive {
                    prev: last_id,
                    next: first_id,
                },
            ));
        }

        Ok(())
    }

    pub async fn defensive_nonempty_range<RNG: RangeBounds<u64> + Clone + Debug + Send>(
        &self,
        range: RNG,
    ) -> Result<(), DefensiveError> {
        if !*self.defensive.read().await {
            return Ok(());
        }
        let start = match range.start_bound() {
            Bound::Included(i) => Some(*i),
            Bound::Excluded(i) => Some(*i + 1),
            Bound::Unbounded => None,
        };

        let end = match range.end_bound() {
            Bound::Included(i) => Some(*i),
            Bound::Excluded(i) => Some(*i - 1),
            Bound::Unbounded => None,
        };

        if start.is_none() || end.is_none() {
            return Ok(());
        }

        if start > end {
            return Err(DefensiveError::new(ErrorSubject::Logs, Violation::RangeEmpty {
                start,
                end,
            }));
        }

        Ok(())
    }

    /// Requires a range must be at least half open: (-oo, n] or [n, +oo)
    pub async fn defensive_half_open_range<RNG: RangeBounds<u64> + Clone + Debug + Send>(
        &self,
        range: RNG,
    ) -> Result<(), DefensiveError> {
        if !*self.defensive.read().await {
            return Ok(());
        }

        if let Bound::Unbounded = range.start_bound() {
            return Ok(());
        };

        if let Bound::Unbounded = range.end_bound() {
            return Ok(());
        };

        Err(DefensiveError::new(ErrorSubject::Logs, Violation::RangeNotHalfOpen {
            start: range.start_bound().cloned(),
            end: range.end_bound().cloned(),
        }))
    }

    pub async fn defensive_range_hits_logs<T: AppData, RNG: RangeBounds<u64> + Debug + Send>(
        &self,
        range: RNG,
        logs: &[Entry<T>],
    ) -> Result<(), DefensiveError> {
        if !*self.defensive.read().await {
            return Ok(());
        }

        {
            let want_first = match range.start_bound() {
                Bound::Included(i) => Some(*i),
                Bound::Excluded(i) => Some(*i + 1),
                Bound::Unbounded => None,
            };

            let first = logs.first().map(|x| x.log_id.index);

            if want_first.is_some() && first != want_first {
                return Err(DefensiveError::new(
                    ErrorSubject::LogIndex(want_first.unwrap()),
                    Violation::LogIndexNotFound {
                        want: want_first.unwrap(),
                        got: first,
                    },
                ));
            }
        }

        {
            let want_last = match range.end_bound() {
                Bound::Included(i) => Some(*i),
                Bound::Excluded(i) => Some(*i - 1),
                Bound::Unbounded => None,
            };

            let last = logs.last().map(|x| x.log_id.index);

            if want_last.is_some() && last != want_last {
                return Err(DefensiveError::new(
                    ErrorSubject::LogIndex(want_last.unwrap()),
                    Violation::LogIndexNotFound {
                        want: want_last.unwrap(),
                        got: last,
                    },
                ));
            }
        }

        Ok(())
    }

    pub async fn defensive_apply_log_id_gt_last<D: AppData>(
        &self,
        entries: &[&Entry<D>],
    ) -> Result<(), DefensiveError> {
        if !*self.defensive.read().await {
            return Ok(());
        }

        let last_id = self.sm.read().await.last_applied_log;

        let first_id = entries[0].log_id;
        if first_id < last_id {
            return Err(DefensiveError::new(
                ErrorSubject::Apply(first_id),
                Violation::ApplyNonConsecutive {
                    prev: last_id,
                    next: first_id,
                },
            ));
        }

        Ok(())
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
    fn find_first_membership_log<'a, T, D>(mut it: T) -> Option<(LogId, MembershipConfig)>
    where
        T: 'a + Iterator<Item = &'a Entry<D>>,
        D: AppData,
    {
        it.find_map(|entry| match &entry.payload {
            EntryPayload::ConfigChange(cfg) => Some((entry.log_id, cfg.membership.clone())),
            _ => None,
        })
    }

    /// Go backwards through the log to find the most recent membership config <= `upto_index`.
    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn get_membership_from_log(&self, upto_index: Option<u64>) -> Result<MembershipConfig, StorageError> {
        self.defensive_no_dirty_log().await?;

        let membership_in_log = {
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

        let (_, membership_in_sm) = self.last_applied_state().await?;

        let membership =
            if membership_in_log.as_ref().map(|(id, _)| id.index) > membership_in_sm.as_ref().map(|(id, _)| id.index) {
                membership_in_log
            } else {
                membership_in_sm
            };

        // Create a default one if both are None.

        Ok(match membership {
            Some((_id, cfg)) => cfg,
            None => MembershipConfig::new_initial(self.id),
        })
    }
}

#[async_trait]
impl RaftStorage<ClientRequest, ClientResponse> for MemStore {
    type SnapshotData = Cursor<Vec<u8>>;

    async fn defensive(&self, d: bool) -> bool {
        let mut defensive_flag = self.defensive.write().await;
        *defensive_flag = d;
        d
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_membership_config(&self) -> Result<MembershipConfig, StorageError> {
        self.get_membership_from_log(None).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_initial_state(&self) -> Result<InitialState, StorageError> {
        self.defensive_no_dirty_log().await?;

        let membership = self.get_membership_config().await?;
        let mut hs = self.hs.write().await;
        match &mut *hs {
            Some(inner) => {
                // Search for two place and use the max one,
                // because when a state machine is installed there could be logs
                // included in the state machine that are not cleaned:
                // - the last log id
                // - the last_applied log id in state machine.

                let last_in_log = self.last_id_in_log().await?;
                let (last_applied, _) = self.last_applied_state().await?;

                let last_log_id = max(last_in_log, last_applied);

                Ok(InitialState {
                    last_log_id,
                    last_applied,
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

    #[tracing::instrument(level = "debug", skip(self))]
    async fn save_hard_state(&self, hs: &HardState) -> Result<(), StorageError> {
        self.defensive_incremental_hard_state(hs).await?;

        let mut h = self.hs.write().await;

        *h = Some(hs.clone());
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_log_entries<RNG: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &self,
        range: RNG,
    ) -> Result<Vec<Entry<ClientRequest>>, StorageError> {
        self.defensive_nonempty_range(range.clone()).await?;

        let res = {
            let log = self.log.read().await;
            log.range(range.clone()).map(|(_, val)| val.clone()).collect::<Vec<_>>()
        };

        self.defensive_range_hits_logs(range, &res).await?;

        Ok(res)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn try_get_log_entry(&self, log_index: u64) -> Result<Option<Entry<ClientRequest>>, StorageError> {
        let log = self.log.read().await;
        Ok(log.get(&log_index).cloned())
    }

    async fn last_id_in_log(&self) -> Result<LogId, StorageError> {
        let log = self.log.read().await;
        let last = log.iter().last().map(|(_, ent)| ent.log_id).unwrap_or_default();
        Ok(last)
    }

    async fn last_applied_state(&self) -> Result<(LogId, Option<(LogId, MembershipConfig)>), StorageError> {
        let sm = self.sm.read().await;
        Ok((sm.last_applied_log, sm.last_membership.clone()))
    }

    #[tracing::instrument(level = "trace", skip(self, range), fields(range=?range))]
    async fn delete_logs_from<R: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &self,
        range: R,
    ) -> Result<(), StorageError> {
        self.defensive_nonempty_range(range.clone()).await?;
        self.defensive_half_open_range(range.clone()).await?;

        let mut log = self.log.write().await;

        let keys = log.range(range).map(|(k, _v)| *k).collect::<Vec<_>>();
        for key in keys {
            log.remove(&key);
        }

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn append_to_log(&self, entries: &[&Entry<ClientRequest>]) -> Result<(), StorageError> {
        self.defensive_nonempty_input(entries).await?;
        self.defensive_consecutive_input(entries).await?;
        self.defensive_append_log_index_is_last_plus_one(entries).await?;
        self.defensive_append_log_id_gt_last(entries).await?;

        let mut log = self.log.write().await;
        for entry in entries {
            log.insert(entry.log_id.index, (*entry).clone());
        }
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn apply_to_state_machine(
        &self,
        entries: &[&Entry<ClientRequest>],
    ) -> Result<Vec<ClientResponse>, StorageError> {
        self.defensive_nonempty_input(entries).await?;
        self.defensive_apply_index_is_last_applied_plus_one(entries).await?;
        self.defensive_apply_log_id_gt_last(entries).await?;

        let mut sm = self.sm.write().await;
        let mut res = Vec::with_capacity(entries.len());

        for entry in entries {
            tracing::debug!("id:{} replicate to sm index:{}", self.id, entry.log_id.index);

            sm.last_applied_log = entry.log_id;

            match entry.payload {
                EntryPayload::Blank => res.push(ClientResponse(None)),
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
                    sm.last_membership = Some((entry.log_id, mem.membership.clone()));
                    res.push(ClientResponse(None))
                }
            };
        }
        Ok(res)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn do_log_compaction(&self) -> Result<Snapshot<Self::SnapshotData>, StorageError> {
        let (data, last_applied_log);
        let membership_config;

        {
            // Serialize the data of the state machine.
            let sm = self.sm.read().await;
            data = serde_json::to_vec(&*sm)
                .map_err(|e| StorageIOError::new(ErrorSubject::StateMachine, ErrorVerb::Read, e.into()))?;

            last_applied_log = sm.last_applied_log;
            membership_config = sm
                .last_membership
                .clone()
                .map(|(_id, mem)| mem)
                .unwrap_or_else(|| MembershipConfig::new_initial(self.id));
        }

        let snapshot_size = data.len();

        let snapshot_idx = {
            let mut l = self.snapshot_idx.lock().unwrap();
            *l += 1;
            *l
        };

        let meta;
        {
            let mut current_snapshot = self.current_snapshot.write().await;

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
    async fn begin_receiving_snapshot(&self) -> Result<Box<Self::SnapshotData>, StorageError> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn finalize_snapshot_installation(
        &self,
        meta: &SnapshotMeta,
        snapshot: Box<Self::SnapshotData>,
    ) -> Result<StateMachineChanges, StorageError> {
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
            let new_sm: MemStoreStateMachine = serde_json::from_slice(&new_snapshot.data).map_err(|e| {
                StorageIOError::new(
                    ErrorSubject::Snapshot(new_snapshot.meta.clone()),
                    ErrorVerb::Read,
                    e.into(),
                )
            })?;
            let mut sm = self.sm.write().await;
            *sm = new_sm;
        }

        // Update current snapshot.
        let mut current_snapshot = self.current_snapshot.write().await;
        *current_snapshot = Some(new_snapshot);
        Ok(StateMachineChanges {
            last_applied: Some(meta.last_log_id),
            is_snapshot: true,
        })
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&self) -> Result<Option<Snapshot<Self::SnapshotData>>, StorageError> {
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
