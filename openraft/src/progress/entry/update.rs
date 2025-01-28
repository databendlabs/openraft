use crate::display_ext::DisplayOptionExt;
use crate::engine::EngineConfig;
use crate::progress::entry::ProgressEntry;
use crate::type_config::alias::LogIdOf;
use crate::LogIdOptionExt;
use crate::RaftTypeConfig;

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

    /// Update conflicting log index.
    ///
    /// Conflicting log index is the last found log index on a follower that is not matching the
    /// leader log.
    ///
    /// Usually if follower's data is not lost, `conflict` is always greater than or equal
    /// `matching`. But for testing purpose, a follower is allowed to clean its data and wait
    /// for leader to replicate all data to it.
    ///
    /// To allow a follower to clean its data, set the config [`Config::allow_log_reversion`] .
    ///
    /// [`Config::allow_log_reversion`]: `crate::config::Config::allow_log_reversion`
    pub(crate) fn update_conflicting(&mut self, conflict: u64) {
        tracing::debug!(
            "update_conflict: current progress_entry: {}; conflict: {}",
            self.entry,
            conflict
        );

        self.entry.inflight.conflict(conflict);

        debug_assert!(conflict < self.entry.searching_end);
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

                self.entry.matching = None;
                self.entry.allow_log_reversion = false;
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

    pub(crate) fn update_matching(&mut self, matching: Option<LogIdOf<C>>) {
        tracing::debug!(
            "update_matching: current progress_entry: {}; matching: {}",
            self.entry,
            matching.display()
        );

        self.entry.inflight.ack(matching.clone());

        debug_assert!(matching.as_ref() >= self.entry.matching());
        self.entry.matching = matching;

        let matching_next = self.entry.matching().next_index();
        self.entry.searching_end = std::cmp::max(self.entry.searching_end, matching_next);
    }
}
