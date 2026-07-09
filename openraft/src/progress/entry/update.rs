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

    /// Update the conflicting log index for this follower.
    ///
    /// The conflicting log index is the last log index found on a follower that does not match
    /// the leader's log at that position.
    ///
    /// If `inflight_id` is `Some`, the inflight state is reset because the response corresponds
    /// to a replication request with log payload. If `None`, the response is from an RPC without
    /// payload (e.g., heartbeat), and inflight state is not modified.
    ///
    /// Normally, the `conflict` index should be greater than or equal to the `matching` index
    /// when follower data is intact. However, for testing purposes, a follower may clean its
    /// data and require the leader to replicate all data from the beginning.
    ///
    /// To allow follower log reversion, enable [`Config::allow_log_reversion`].
    ///
    /// When log reversion is allowed and the conflict is before the last known matching log,
    /// this method resets `matching` to `None`. This lowers the progress value, so the caller
    /// must apply it via `VecProgress::reset_entry_with()`, which moves the entry down to keep
    /// the progress ordering while keeping the already quorum-accepted value: a value accepted
    /// by a quorum must never be withdrawn.
    ///
    /// [`Config::allow_log_reversion`]: `crate::config::Config::allow_log_reversion`
    pub(crate) fn update_conflicting(&mut self, conflict: u64, inflight_id: Option<InflightId>) {
        tracing::debug!(
            "update_conflict: current progress_entry: {}; conflict: {}",
            self.entry,
            conflict
        );

        // The inflight may be None if the conflict is caused by a heartbeat response.
        if let Some(inflight_id) = inflight_id {
            self.entry.inflight.conflict(conflict, inflight_id);
        }

        if conflict >= self.entry.searching_end {
            tracing::debug!(
                "conflict {} >= searching_end {}; no need to update",
                conflict,
                self.entry.searching_end
            );
            return;
        }

        self.entry.searching_end = conflict;

        // An already matching log id is found lost:
        //
        // - If log reversion is allowed, just restart the binary search from the beginning.
        // - Otherwise, panic it.

        let allow_reset = self.entry.allow_log_reversion || self.engine_config.allow_log_reversion;

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
                self.entry.allow_log_reversion = false;

                // Reset pipeline mode when logs are reverted
                if self.entry.inflight.is_logs_since() {
                    self.entry.inflight = Inflight::None;
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
    }

    /// Update the matching log id for this follower when replication succeeds.
    ///
    /// If `inflight_id` is `Some`, the inflight state is acknowledged because the response
    /// corresponds to a replication request with log payload. If `None`, the response is from
    /// an RPC without payload (e.g., heartbeat), and inflight state is not modified.
    pub(crate) fn update_matching(&mut self, matching: Option<LogIdOf<C>>, inflight_id: Option<InflightId>) {
        tracing::debug!(
            "update_matching: current progress_entry: {}; matching: {}",
            self.entry,
            matching.display()
        );

        if let Some(inflight_id) = inflight_id {
            let applied = self.entry.inflight.ack(matching.clone(), inflight_id);
            if !applied {
                // Stale payload ack: a newer request superseded this one, or a concurrent log
                // reversion cleared the inflight to `None` (see `update_conflicting`). Do not
                // advance `matching`: the follower no longer holds the acked logs, and counting
                // them as matched could form a false quorum over data the follower has reverted.
                return;
            }
        }

        // If it is not a response of an actual replication(such as replicating commit log id),
        // it may not indicate an actual progress.
        if inflight_id.is_none() && matching <= self.entry.matching {
            return;
        }

        debug_assert!(matching.as_ref() >= self.entry.matching());
        self.entry.matching = matching;

        let matching_next = self.entry.matching().next_index();
        self.entry.searching_end = std::cmp::max(self.entry.searching_end, matching_next);
    }
}
