use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use futures::FutureExt;

use crate::LogId;
use crate::LogIdOptionExt;
use crate::RaftLogReader;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscSender;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::notification::Notification;
use crate::display_ext::display_option::DisplayOptionExt;
use crate::entry::RaftEntry;
use crate::entry::raft_entry_ext::RaftEntryExt;
use crate::error::StorageIOResult;
use crate::log_id_range::LogIdRange;
use crate::network::Backoff;
use crate::progress::inflight_id::InflightId;
use crate::raft::AppendEntriesRequest;
use crate::raft_state::IOId;
use crate::replication::event_watcher::EventWatcher;
use crate::replication::payload::Payload;
use crate::replication::replication_context::ReplicationContext;
use crate::storage::RaftLogStorage;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::EntryOf;
use crate::type_config::alias::LogIdOf;
use crate::vote::RaftVote;

/// Mutable state for generating AppendEntries requests in a replication stream.
///
/// This struct holds the log reader and tracks what log entries need to be sent next.
/// It is protected by a mutex and shared between the stream generator and the
/// replication task that updates `log_id_range` when new entries arrive.
pub(crate) struct StreamState<C, LS>
where
    C: RaftTypeConfig,
    LS: RaftLogStorage<C>,
{
    pub(crate) replication_context: ReplicationContext<C>,

    pub(crate) event_watcher: EventWatcher<C>,

    /// The [`RaftLogStorage::LogReader`] interface.
    pub(crate) log_reader: LS::LogReader,

    /// The range of log entries to replicate: `(prev_log_id, last_log_id]`.
    ///
    /// Set to `None` when all entries have been sent.
    pub(crate) payload: Option<Payload<C>>,

    pub(crate) inflight_id: Option<InflightId>,

    /// The leader's committed log id to send in AppendEntries requests.
    pub(crate) leader_committed: Option<LogId<C>>,

    /// The backoff policy if an [`Unreachable`](`crate::error::Unreachable`) error is returned.
    /// It will be reset to `None` when a successful response is received.
    pub(crate) backoff: Arc<Mutex<Option<Backoff>>>,
}

impl<C, LS> StreamState<C, LS>
where
    C: RaftTypeConfig,
    LS: RaftLogStorage<C>,
{
    /// Generates the next AppendEntries request from the current log range.
    ///
    /// Returns `None` when there are no more entries to send or on storage error.
    /// After each call, `log_id_range` is updated to exclude the sent entries.
    pub(crate) async fn next_request(&mut self) -> Option<AppendEntriesRequest<C>> {
        // The initial log_id_range may be empty range, for sync a commit log id.
        // In this case, still send one RPC, and set log_id_range in `update_log_id_range()`
        let log_id_range = self.get_log_id_range().await?;

        tracing::debug!("{}: log_id_range: {}", func_name!(), log_id_range);

        let res = self.read_log_entries(log_id_range).await;
        let (entries, sending_range) = match res {
            Ok(x) => x,
            Err(sto_err) => {
                tracing::error!("{} replication to target={}", sto_err, self.replication_context.target);

                self.replication_context.tx_notify.send(Notification::StorageError { error: sto_err }).await.ok();
                return None;
            }
        };

        let belonging_leader = self.replication_context.leader_vote.leader_id().clone();
        let accepted_io: IOId<C> = self.event_watcher.io_accepted_rx.borrow_watched().clone();
        let current_leader = accepted_io.leader_id().clone();
        if current_leader != belonging_leader {
            tracing::info!(
                "Leader changed from {} to {}, quit replication",
                belonging_leader,
                current_leader
            );
            return None;
        }

        self.update_log_id_range(sending_range.last);

        let payload = AppendEntriesRequest {
            vote: self.replication_context.leader_vote.clone().into_vote(),
            prev_log_id: sending_range.prev.clone(),
            leader_commit: self.event_watcher.committed_rx.borrow_watched().clone(),
            entries,
        };

        self.replication_context.replicate_batch.record(payload.entries.len() as u64);

        tracing::debug!("next_request: AppendEntries: {}", payload);

        self.backoff_if_enabled().await;

        Some(payload)
    }

    /// Return None if no more data to send.
    async fn get_log_id_range(&mut self) -> Option<LogIdRange<C>> {
        let payload = self.payload.as_ref()?;

        tracing::debug!("pipeline stream payload: {}", payload);

        let prev = match payload {
            Payload::LogIdRange { log_id_range } => return Some(log_id_range.clone()),
            Payload::LogsSince { prev } => prev.clone(),
        };

        // pipeline mode:

        loop {
            let current: IOId<C> = self.event_watcher.io_submitted_rx.borrow_watched().clone();
            let last_log_id = current.last_log_id().cloned();

            let committed: Option<LogIdOf<C>> = self.event_watcher.committed_rx.borrow_watched().clone();

            tracing::debug!(
                "building next entries range to replicate: current last_log_id: {}, current committed: {}",
                last_log_id.display(),
                committed.display()
            );

            if last_log_id > prev || committed > self.leader_committed {
                self.leader_committed = committed;
                return Some(LogIdRange::new(prev, last_log_id));
            } else {
                let data_change = self.event_watcher.replicate_rx.changed();
                let io_change = self.event_watcher.io_submitted_rx.changed();
                let committed_change = self.event_watcher.committed_rx.changed();
                let cancel = self.replication_context.cancel_rx.changed();

                futures::select! {
                    _data_changed = data_change.fuse() => {
                        let new_data = self.event_watcher.replicate_rx.borrow_watched().clone();
                        if Some(new_data.inflight_id) != self.inflight_id {
                            tracing::info!("current inflight_id: {} received payload with new inflight_id: {}, quit", self.inflight_id.display(), new_data.inflight_id);
                            return None;
                        }
                    }
                    _io_changed = io_change.fuse() => {
                        tracing::debug!("io_submitted_rx changed");
                        // Continue
                    }
                    _committed_change = committed_change.fuse() => {
                        tracing::debug!("committed_rx changed");
                        // `committed` may be triggered even when the value does not change.
                        // in which scenario, it is for replication committed log id,
                        // thus we just emit an RPC once committed receiver is notified.
                        self.leader_committed = self.event_watcher.committed_rx.borrow_watched().clone();
                        return Some(LogIdRange::new(prev, last_log_id));
                    }
                    cancel_res = cancel.fuse() => {
                        tracing::info!("Replication Stream is canceled, res: {:?}, when:(get_log_id_range:wait-for-changed)", cancel_res);
                        return None;
                    }
                }
            }
        }
    }

    /// Waits for the backoff duration if backoff is enabled, or returns immediately.
    async fn backoff_if_enabled(&mut self) {
        let sleep_duration = {
            let mut backoff = self.backoff.lock().unwrap();
            let Some(backoff) = &mut *backoff else { return };

            backoff.next().unwrap_or_else(|| Duration::from_millis(500))
        };

        let sleep = C::sleep(sleep_duration);
        let cancel = self.replication_context.cancel_rx.changed();

        tracing::debug!("backoff timeout: {:?}", sleep_duration);

        futures::select! {
            _ = sleep.fuse() => {
                tracing::debug!("backoff timeout");
            }
            cancel_res = cancel.fuse() => {
                tracing::info!("Replication Stream is canceled, res: {:?}, when:(backoff_if_enabled:wait-for-changed)", cancel_res);
            }
        }
    }

    /// Updates `log_id_range` after sending entries up to `matching`.
    ///
    /// Sets `log_id_range` to `None` when all entries have been sent.
    fn update_log_id_range(&mut self, matching: Option<LogIdOf<C>>) {
        let Some(payload) = self.payload.as_mut() else {
            return;
        };

        payload.update_matching(matching);

        if payload.len() == Some(0) {
            self.payload = None;
        }
    }

    /// Reads log entries from storage for the given range.
    ///
    /// Returns the entries and the actual range covered (may be smaller than requested
    /// due to `limited_get_log_entries`).
    async fn read_log_entries(
        &mut self,
        log_id_range: LogIdRange<C>,
    ) -> Result<(Vec<EntryOf<C>>, LogIdRange<C>), StorageError<C>> {
        tracing::debug!("read_log_entries: log_id_range: {}", log_id_range);

        // Series of logs to send, and the last log id to send
        let rng = &log_id_range;

        // The log index start and end to send.
        let (start, end) = {
            let start = rng.prev.next_index();
            let end = rng.last.next_index();

            (start, end)
        };

        if start == end {
            // Heartbeat RPC, no logs to send, last log id is the same as prev_log_id
            let r = LogIdRange::new(rng.prev.clone(), rng.prev.clone());
            Ok((vec![], r))
        } else {
            let max_entries = self.replication_context.config.max_payload_entries;
            let end = std::cmp::min(end, start + max_entries);

            // limited_get_log_entries will return logs smaller than the range [start, end).
            let logs = self.log_reader.limited_get_log_entries(start, end).await.sto_read_logs()?;

            // Handle empty result gracefully: treat as heartbeat.
            // This violates the API contract but we don't panic.
            // We sleep briefly to avoid a tight loop since the log_id_range won't advance.
            if logs.is_empty() {
                let sleep_duration = Duration::from_millis(10);
                tracing::warn!(
                    "limited_get_log_entries({}, {}) returned empty; \
                     this violates the API contract but is handled gracefully as a heartbeat. \
                     Sleeping {:?} to avoid tight loop.",
                    start,
                    end,
                    sleep_duration
                );
                C::sleep(sleep_duration).await;
                let r = LogIdRange::new(rng.prev.clone(), rng.prev.clone());
                return Ok((vec![], r));
            }

            let first = logs.first().map(|ent| ent.ref_log_id()).unwrap();
            let last = logs.last().map(|ent| ent.log_id()).unwrap();

            debug_assert!(
                logs.len() <= (end - start) as usize,
                "expect logs ⊆ [{}..{}) but got {} entries, first: {}, last: {}",
                start,
                end,
                logs.len(),
                first,
                last
            );

            let r = LogIdRange::new(rng.prev.clone(), Some(last));
            Ok((logs, r))
        }
    }
}
