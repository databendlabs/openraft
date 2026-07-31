//! Internal storage adapter
//!
//! This module bridges the user's [`EzApp`] and [`EzStorage`] traits
//! to OpenRaft's [`RaftLogStorage`] and [`RaftStateMachine`] traits.
//!
//! Users don't interact with this module directly - it's used internally by [`crate::EzRaft`].

use std::fmt::Debug;
use std::io::Cursor;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::ops::RangeBounds;
use std::sync::Arc;

use futures::StreamExt;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::Membership;
use openraft::OptionalSend;
use openraft::RaftLogReader;
use openraft::RaftSnapshotBuilder;
use openraft::RaftTypeConfig;
use openraft::Snapshot;
use openraft::SnapshotMeta;
use openraft::StoredMembership;
use openraft::alias::LogIdOf;
use openraft::alias::StoredMembershipOf;
use openraft::log_id::LogIndexOptionExt;
use openraft::log_id::RaftLogId;
use openraft::storage::EntryResponder;
use openraft::storage::IOFlushed;
use openraft::storage::LogState;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftStateMachine;
use tokio::sync::Mutex;

use crate::app::EzApp;
use crate::meta::EzMeta;
use crate::snapshot::EzSnapshot;
use crate::snapshot::EzSnapshotData;
use crate::snapshot::EzSnapshotMeta;
use crate::storage::EzStorage;
use crate::storage::Loaded;
use crate::storage::Persist;
use crate::type_config::OpenRaftTypes;

/// Internal storage state protected by single mutex
pub struct StorageWithCache<T>
where T: EzApp
{
    pub storage: Box<dyn EzStorage<T>>,
    pub cached_meta: EzMeta,

    /// The snapshot last written or loaded, kept so that serving one to a lagging follower does
    /// not re-run the startup-only [`EzStorage::load`].
    pub cached_snapshot: Option<EzSnapshot>,
}

/// Internal state machine wrapper that tracks Raft metadata
/// alongside the user's application
pub struct StateMachineState<T>
where T: EzApp
{
    /// User's application: the state machine value itself
    pub app: T,

    /// Last log ID applied to the state machine
    pub last_applied: Option<LogIdOf<OpenRaftTypes<T>>>,

    /// Last membership applied to the state machine
    pub membership: StoredMembershipOf<OpenRaftTypes<T>>,
}

/// Internal storage adapter
///
/// Bridges user's `EzApp` and `EzStorage` to OpenRaft's storage traits.
///
/// Only metadata is cached in memory - logs are read from user storage on demand.
pub struct StorageAdapter<T>
where T: EzApp
{
    pub(crate) storage: Arc<Mutex<StorageWithCache<T>>>,
    pub(crate) sm_state: Arc<Mutex<StateMachineState<T>>>,
}

impl<T> StorageAdapter<T>
where T: EzApp
{
    /// Create a new storage adapter and load initial metadata
    pub async fn new(mut user_storage: impl EzStorage<T>, app: T) -> Result<Self, std::io::Error> {
        // Load initial metadata and snapshot
        let Loaded {
            meta: cached_meta,
            snapshot,
        } = user_storage.load().await?;

        let mut app = app;

        // Initialize state machine state from snapshot or defaults.
        //
        // The snapshot data must be restored here, not just its position: reporting
        // `last_applied` at the snapshot makes openraft re-apply only the log tail after it, and
        // skip installing this snapshot itself.
        let (last_applied, last_membership) = match &snapshot {
            Some(snap) => {
                app = serde_json::from_slice(snap.snapshot.get_ref())?;
                (snap.meta.last_log_id, snap.meta.last_membership.clone())
            }
            None => (None, StoredMembership::new(None, Membership::default())),
        };

        let storage = StorageWithCache {
            storage: Box::new(user_storage),
            cached_meta,
            cached_snapshot: snapshot,
        };

        let sm_state = StateMachineState {
            app,
            last_applied,
            membership: last_membership,
        };

        Ok(Self {
            storage: Arc::new(Mutex::new(storage)),
            sm_state: Arc::new(Mutex::new(sm_state)),
        })
    }

    /// Update metadata and persist to storage
    pub async fn save_meta(&self, f: impl FnOnce(&mut EzMeta)) -> Result<(), std::io::Error> {
        let mut state = self.storage.lock().await;
        f(&mut state.cached_meta);
        let update = Persist::Meta(state.cached_meta.clone());
        state.storage.persist(update).await
    }

    /// Get the current node_id from cached metadata
    pub async fn node_id(&self) -> Option<u64> {
        self.storage.lock().await.cached_meta.node_id
    }
}

// Implement RaftLogStorage for Arc<StorageAdapter>
impl<T> RaftLogStorage<OpenRaftTypes<T>> for Arc<StorageAdapter<T>>
where T: EzApp
{
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<OpenRaftTypes<T>>, std::io::Error> {
        let state = self.storage.lock().await;
        let last = state.cached_meta.last_log_id.map(|(t, i)| LogId::new_term_index(t, i));
        let last_purged = state.cached_meta.last_purged.map(|(t, i)| LogId::new_term_index(t, i));

        Ok(LogState {
            last_log_id: last,
            last_purged_log_id: last_purged,
        })
    }

    async fn save_vote(&mut self, vote: &<OpenRaftTypes<T> as RaftTypeConfig>::Vote) -> Result<(), std::io::Error> {
        self.save_meta(|m| m.vote = Some(*vote)).await
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<OpenRaftTypes<T>>) -> Result<(), std::io::Error>
    where
        I: IntoIterator<Item = <OpenRaftTypes<T> as RaftTypeConfig>::Entry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        // One lock for the whole batch and the metadata that describes it, so no reader can
        // observe a log that reaches past the last_log_id recorded for it.
        let res = async {
            let mut state = self.storage.lock().await;

            let mut last_log_id = None;

            // Save all log entries
            for entry in entries {
                last_log_id = Some(entry.log_id);
                state.storage.persist(Persist::LogEntry(entry)).await?;
            }

            // Update metadata once with the last entry's log_id
            if let Some(log_id) = last_log_id {
                state.cached_meta.last_log_id = Some(log_id);
                let update = Persist::Meta(state.cached_meta.clone());
                state.storage.persist(update).await?;
            }

            Ok::<_, std::io::Error>(())
        }
        .await;

        // openraft is waiting on this callback; returning the error alone would leave the append
        // in flight forever.
        match res {
            Ok(()) => {
                callback.io_completed(Ok(()));
                Ok(())
            }
            Err(e) => {
                callback.io_completed(Err(std::io::Error::other(e.to_string())));
                Err(e)
            }
        }
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogIdOf<OpenRaftTypes<T>>>) -> Result<(), std::io::Error> {
        let from = last_log_id.map(|id| id.index).next_index();
        {
            let mut state = self.storage.lock().await;
            state.storage.persist(Persist::DeleteLogs { from, to: u64::MAX }).await?;
        }

        self.save_meta(|m| {
            m.last_log_id = last_log_id.map(|id| id.to_type());
        })
        .await
    }

    async fn purge(&mut self, log_id: LogIdOf<OpenRaftTypes<T>>) -> Result<(), std::io::Error> {
        {
            let mut state = self.storage.lock().await;
            state
                .storage
                .persist(Persist::DeleteLogs {
                    from: 0,
                    to: log_id.index + 1,
                })
                .await?;
        }

        self.save_meta(|m| m.last_purged = Some(log_id.to_type())).await
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }
}

// Implement RaftLogReader for Arc<StorageAdapter>
impl<T> RaftLogReader<OpenRaftTypes<T>> for Arc<StorageAdapter<T>>
where T: EzApp
{
    async fn read_vote(&mut self) -> Result<Option<<OpenRaftTypes<T> as RaftTypeConfig>::Vote>, std::io::Error> {
        let state = self.storage.lock().await;
        Ok(state.cached_meta.vote)
    }

    async fn try_get_log_entries<RB>(
        &mut self,
        range: RB,
    ) -> Result<Vec<<OpenRaftTypes<T> as RaftTypeConfig>::Entry>, std::io::Error>
    where
        RB: RangeBounds<u64> + Clone + Debug + OptionalSend,
    {
        // Held until the entries have been read: a purge landing in between would leave the
        // clamped range pointing at entries user storage has already deleted.
        let mut state = self.storage.lock().await;

        // Available log range: [lo, hi)
        let lo = state.cached_meta.last_purged.map(|(_, i)| i).next_index();
        let hi = state.cached_meta.last_log_id.map(|(_, i)| i).next_index();

        let start = match range.start_bound() {
            std::ops::Bound::Included(&x) => x,
            std::ops::Bound::Excluded(&x) => x + 1,
            std::ops::Bound::Unbounded => 0,
        };

        let end = match range.end_bound() {
            std::ops::Bound::Included(&x) => x + 1,
            std::ops::Bound::Excluded(&x) => x,
            std::ops::Bound::Unbounded => hi,
        };

        // Clamp to available range
        let start = std::cmp::max(start, lo);
        let end = std::cmp::min(end, hi);

        if start >= end {
            return Ok(Vec::new());
        }

        // Load only the requested range from user storage
        state.storage.read_logs(start, end).await
    }
}

// Implement RaftStateMachine for Arc<StorageAdapter>
impl<T> RaftStateMachine<OpenRaftTypes<T>> for Arc<StorageAdapter<T>>
where T: EzApp
{
    type SnapshotData = EzSnapshotData;

    type SnapshotBuilder = Self;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogIdOf<OpenRaftTypes<T>>>, StoredMembershipOf<OpenRaftTypes<T>>), std::io::Error> {
        let sm = self.sm_state.lock().await;
        Ok((sm.last_applied, sm.membership.clone()))
    }

    async fn apply<Strm>(&mut self, entries: Strm) -> Result<(), std::io::Error>
    where Strm: futures::Stream<Item = Result<EntryResponder<OpenRaftTypes<T>>, std::io::Error>> + OptionalSend + Unpin
    {
        let mut sm = self.sm_state.lock().await;

        let mut entries = entries;
        while let Some(res) = entries.next().await {
            let (entry, responder) = res.map_err(std::io::Error::other)?;

            // Update last_applied for every entry
            let (term, index) = entry.log_id;
            let log_id = LogId::new_term_index(term, index);
            sm.last_applied = Some(log_id);

            let resp = match entry.payload {
                EntryPayload::Normal(req) => Some(sm.app.apply(req).await),
                EntryPayload::Membership(membership) => {
                    sm.membership = StoredMembership::new(Some(log_id), membership);
                    None
                }
                EntryPayload::Blank => None,
            };

            if let Some(responder) = responder {
                responder.send(resp);
            }
        }

        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn install_snapshot(
        &mut self,
        snapshot_meta: &EzSnapshotMeta,
        snapshot_data: EzSnapshotData,
    ) -> Result<(), std::io::Error> {
        // Extract snapshot data
        let mut cursor = snapshot_data;
        cursor.seek(SeekFrom::Start(0))?;
        let mut data = Vec::new();
        cursor.read_to_end(&mut data)?;

        // Update storage state
        {
            let mut state = self.storage.lock().await;
            state.storage.persist(Persist::Snapshot(new_snapshot(snapshot_meta, &data))).await?;
            state.cached_snapshot = Some(new_snapshot(snapshot_meta, &data));
        }

        // The snapshot supersedes every log entry it covers, so the log positions move with it.
        // Persist them here instead of leaving the cached copy for whoever writes meta next: a
        // crash in between would leave a snapshot on disk that meta does not account for.
        //
        // Never move a position backwards - the local log may already be ahead of the snapshot.
        let snapshot_log_id = snapshot_meta.last_log_id.map(|id| id.to_type());
        self.save_meta(|m| {
            m.last_log_id = m.last_log_id.max(snapshot_log_id);
            m.last_purged = m.last_purged.max(snapshot_log_id);
        })
        .await?;

        // Update state machine state and restore user state from snapshot
        {
            let mut sm = self.sm_state.lock().await;
            sm.last_applied = snapshot_meta.last_log_id;
            sm.membership = snapshot_meta.last_membership.clone();
            sm.app = serde_json::from_slice(&data)?;
        }

        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<EzSnapshot>, std::io::Error> {
        let state = self.storage.lock().await;
        Ok(state.cached_snapshot.as_ref().map(|snap| new_snapshot(&snap.meta, snap.snapshot.get_ref())))
    }
}

// Implement RaftSnapshotBuilder for Arc<StorageAdapter>
impl<T> RaftSnapshotBuilder<OpenRaftTypes<T>> for Arc<StorageAdapter<T>>
where T: EzApp
{
    type SnapshotData = EzSnapshotData;

    async fn build_snapshot(&mut self) -> Result<EzSnapshot, std::io::Error> {
        // Get current state machine state and build snapshot data
        let (last_applied, last_membership, snapshot_data) = {
            let sm = self.sm_state.lock().await;
            let data = serde_json::to_vec(&sm.app)?;
            (sm.last_applied, sm.membership.clone(), data)
        };

        let snapshot_id = match last_applied {
            Some(log_id) => format!("{}-{}", log_id.leader_id.term, log_id.index),
            None => "0-0".to_string(),
        };

        let snapshot_meta = SnapshotMeta {
            last_log_id: last_applied,
            last_membership,
            snapshot_id,
        };

        // Persist before returning: openraft purges logs covered by this snapshot right after,
        // and a durable purge point with no durable snapshot is an unrecoverable state.
        {
            let mut state = self.storage.lock().await;
            state.storage.persist(Persist::Snapshot(new_snapshot(&snapshot_meta, &snapshot_data))).await?;
            state.cached_snapshot = Some(new_snapshot(&snapshot_meta, &snapshot_data));
        }

        Ok(new_snapshot(&snapshot_meta, &snapshot_data))
    }
}

/// Build a [`EzSnapshot`] from its parts
fn new_snapshot(meta: &EzSnapshotMeta, data: &[u8]) -> EzSnapshot {
    Snapshot {
        meta: meta.clone(),
        snapshot: Cursor::new(data.to_vec()),
    }
}
