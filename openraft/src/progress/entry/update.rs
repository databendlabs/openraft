use display_more::DisplayOptionExt;

use crate::LogIdOptionExt;
use crate::RaftTypeConfig;
use crate::engine::EngineConfig;
use crate::progress::Inflight;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight_id::InflightId;
use crate::type_config::alias::LogIdOf;

/// It implements updating operations for a [`ProgressEntry`]
pub(crate) struct Updater<'a, C>
where C: RaftTypeConfig
{
    engine_config: &'a EngineConfig<C>,
    entry: &'a mut ProgressEntry<C>,
}

impl<'a, C> Updater<'a, C>
where C: RaftTypeConfig
{
    pub(crate) fn new(engine_config: &'a EngineConfig<C>, entry: &'a mut ProgressEntry<C>) -> Self {
        Self { engine_config, entry }
    }

    /// Update the progress of this follower with a conflict: the follower rejected a request
    /// because its log does not hold the request's `prev_log_id`, whose index is `conflict`.
    ///
    /// The follower holds another log at `conflict`, or no log there. Either way, the last log it
    /// shares with the leader lies below `conflict`, so `searching_end` drops to `conflict`. A
    /// conflict at or above `searching_end` narrows nothing.
    ///
    /// `inflight_id` names the tracked operation that the response answers. It does not tell
    /// whether the request carried entries: a log stream's request that only sends the commit
    /// index carries the stream's ID. With `Some`, the conflict ends that operation, see
    /// [`Inflight::conflict()`]. The ID of an operation no longer in flight makes the response
    /// stale, and a stale response changes nothing. With `None`, as for a heartbeat or for an
    /// update of the commit index that the replication task sends on its own, the conflict ends
    /// no operation, but it still narrows the search, and the reset below may drop the operation
    /// in flight.
    ///
    /// While the follower's log is intact, the follower holds `matching`, so
    /// `conflict >= matching.next_index()`. A lower conflict means that the follower lost a log it
    /// had matched: its log reverted, such as after its data was wiped. This includes a conflict
    /// at the index of `matching` itself, which a heartbeat reports, because it sends the known
    /// `matching` as its `prev_log_id`.
    ///
    /// [`Config::allow_log_reversion`] allows every reversion, and
    /// [`Trigger::allow_next_revert()`] allows the next reversion of one target. An allowed
    /// reversion resets `matching` to `None`, so that the leader searches again below `conflict`,
    /// and it clears the target's one-shot permission. A conflict that reverts nothing keeps the
    /// permission. The reset lowers the progress value, so the caller must apply it via
    /// `VecProgress::reset_entry_with()`, which moves the entry down to keep the progress ordering
    /// while keeping the already quorum-accepted value: a value accepted by a quorum must never be
    /// withdrawn.
    ///
    /// The reset also drops the operation in flight: a probe, a log stream, or a snapshot
    /// transfer. An answer that the follower sent before it lost its log carries the operation's
    /// ID, and acknowledging it would raise `matching` to logs the follower no longer holds. A
    /// pending [`Inflight::Clear`] stays: no follower answers it, and it holds back the purge of
    /// the logs that a dropped plan's stream may still read.
    ///
    /// A reversion that is not allowed fails a `debug_assert!`. A build without debug assertions
    /// does not panic: it keeps `matching`, and `searching_end` ends below `matching.next_index()`.
    ///
    /// Returns `false` only for a stale response, which changes nothing.
    ///
    /// [`Config::allow_log_reversion`]: `crate::config::Config::allow_log_reversion`
    /// [`Trigger::allow_next_revert()`]: crate::raft::trigger::Trigger::allow_next_revert
    pub(crate) fn update_conflicting(&mut self, conflict: u64, inflight_id: Option<InflightId>) -> bool {
        tracing::debug!(
            "update_conflict: current progress_entry: {}; conflict: {}",
            self.entry,
            conflict
        );

        // The inflight may be None if the conflict is caused by a heartbeat response.
        if let Some(inflight_id) = inflight_id {
            let applied = self.entry.data.inflight.conflict(conflict, inflight_id);
            if !applied {
                return false;
            }
        }

        if conflict >= self.entry.data.searching_end {
            tracing::debug!(
                "conflict {} >= searching_end {}; no need to update",
                conflict,
                self.entry.data.searching_end
            );
            return true;
        }

        self.entry.data.searching_end = conflict;

        // A conflict below `matching.next_index()` means the follower lost a log it had matched:
        //
        // - If log reversion is allowed, reset `matching` and search again below `conflict`.
        // - Otherwise, fail the `debug_assert!` below. Without debug assertions, keep `matching`.

        let allow_reset = self.entry.data.allow_log_reversion || self.engine_config.allow_log_reversion;

        if allow_reset {
            if conflict < self.entry.matching().next_index() {
                tracing::warn!(
                    "conflict {} < last matching {}: \
                    follower log is reverted; \
                    with 'allow_log_reversion' enabled, this is allowed.",
                    conflict,
                    self.entry.matching().display(),
                );

                // This is the only place replication progress is allowed to move backward.
                // The caller applies it via `VecProgress::reset_entry_with()`, which restores
                // the progress ordering without lowering the quorum-accepted value.
                self.entry.matching = None;
                self.entry.data.allow_log_reversion = false;

                // Drop the operation in flight, so that no answer to it raises `matching` again.
                // `ReplicationHandler::update_conflicting()` turns a dropped log plan into an
                // `Inflight::Clear`, which holds back the purge until its stream stops reading.
                match self.entry.data.inflight {
                    Inflight::Probe { .. } | Inflight::LogsSince { .. } | Inflight::Snapshot { .. } => {
                        self.entry.data.inflight = Inflight::None;
                    }
                    Inflight::None | Inflight::Clear { .. } => {}
                }
            }
        } else {
            debug_assert!(
                conflict >= self.entry.matching().next_index(),
                "follower log reversion is not allowed \
                without `allow_log_reversion` enabled; \
                matching: {}; conflict: {}",
                self.entry.matching().display(),
                conflict
            );
        }

        true
    }

    /// Update the matching log id for this follower with a success that answers the tracked
    /// operation `inflight_id`.
    ///
    /// The success acknowledges the operation, and may complete it, only while the operation is in
    /// flight, see [`Inflight::ack()`]. A success for any other operation changes nothing. Whether
    /// the request carried entries does not matter: a log stream's request that only sends the
    /// commit index carries the stream's ID too.
    ///
    /// A success that answers no tracked operation, such as the answer to an update of the commit
    /// index that the replication task sends on its own, never updates the matching log id: it may
    /// report a log that the follower has lost since it answered, and
    /// [`ReplicationHandler::update_progress()`] drops it.
    ///
    /// [`ReplicationHandler::update_progress()`]: crate::engine::handler::replication_handler::ReplicationHandler::update_progress
    pub(crate) fn update_matching(&mut self, matching: Option<LogIdOf<C>>, inflight_id: InflightId) {
        tracing::debug!(
            "update_matching: current progress_entry: {}; matching: {}",
            self.entry,
            matching.display()
        );

        let applied = self.entry.data.inflight.ack(matching.clone(), inflight_id);
        if !applied {
            // Stale ack: the operation it answers is no longer in flight, as when a newer request
            // superseded it, or a log reversion dropped it (see `update_conflicting`). Do not
            // advance `matching`: the follower may have lost the acked logs since it answered, and
            // counting them as matched could form a false quorum over data it has reverted.
            return;
        }

        // An acknowledgement may clear inflight without advancing matching.
        if matching <= self.entry.matching {
            return;
        }

        debug_assert!(matching.is_some(), "a valid update can never set matching to None");
        debug_assert!(matching.as_ref() >= self.entry.matching());
        self.entry.matching = matching;

        let matching_next = self.entry.matching().next_index();
        self.entry.data.searching_end = std::cmp::max(self.entry.data.searching_end, matching_next);
    }
}
