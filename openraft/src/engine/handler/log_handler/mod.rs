use crate::LogIdOptionExt;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::log_id::option_ref_log_id_ext::OptionRefLogIdExt;
use crate::raft_state::LogStateReader;
use crate::type_config::alias::LogIdOf;

#[cfg(test)]
mod calc_purge_upto_test;
#[cfg(test)]
mod purge_log_test;

/// Handle raft-log related operations
pub(crate) struct LogHandler<'x, C>
where C: RaftTypeConfig
{
    pub(crate) config: &'x mut EngineConfig<C>,
    pub(crate) state: &'x mut RaftState<C>,
    pub(crate) output: &'x mut EngineOutput<C>,
}

impl<C> LogHandler<'_, C>
where C: RaftTypeConfig
{
    /// Purge log entries up to `RaftState.purge_upto()`, inclusive.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn purge_log(&mut self) {
        let st = &mut self.state;
        let purge_upto = st.purge_upto();

        tracing::info!(
            last_purged_log_id = display(st.last_purged_log_id().display()),
            purge_upto = display(purge_upto.display()),
            "purge_log"
        );

        if purge_upto <= st.last_purged_log_id() {
            return;
        }

        let upto = purge_upto.unwrap().clone();

        st.purge_log(&upto);
        self.output.push_command(Command::PurgeLog { upto });
    }

    /// Update the next log id to purge up to, if more logs can be purged, according to the
    /// configured policy.
    ///
    /// This method is called after building a snapshot because openraft only purges logs that are
    /// already included in the snapshot.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn schedule_policy_based_purge(&mut self) {
        if let Some(purge_upto) = self.calc_purge_upto() {
            self.update_purge_upto(purge_upto);
        }
    }

    /// Update the log id it expects to purge up to. It won't trigger the purge immediately.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_purge_upto(&mut self, purge_upto: LogIdOf<C>) {
        debug_assert!(self.state.purge_upto() <= Some(&purge_upto));
        self.state.purge_upto = Some(purge_upto);
    }

    /// Calculate the log id up to which to purge, inclusive.
    ///
    /// Only logs included in the snapshot will be purged.
    /// It may return None if there is no log to purge.
    ///
    /// `max_keep` specifies the number of applied logs to keep.
    /// `max_keep==0` means every applied log can be purged.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn calc_purge_upto(&self) -> Option<LogIdOf<C>> {
        let st = &self.state;
        let max_keep = self.config.max_in_snapshot_log_to_keep;
        let batch_size = self.config.purge_batch_size;

        let purge_end = self.state.snapshot_meta.last_log_id.next_index().saturating_sub(max_keep);

        tracing::debug!(
            snapshot_last_log_id = debug(self.state.snapshot_meta.last_log_id.clone()),
            max_keep,
            "try purge: (-oo, {})",
            purge_end
        );

        if st.last_purged_log_id().next_index() + batch_size > purge_end {
            tracing::debug!(
                snapshot_last_log_id = debug(self.state.snapshot_meta.last_log_id.clone()),
                max_keep,
                last_purged_log_id = display(st.last_purged_log_id().display()),
                batch_size,
                purge_end,
                "no need to purge",
            );
            return None;
        }

        let log_id = self.state.log_ids.ref_at(purge_end - 1);
        debug_assert!(
            log_id.is_some(),
            "log id not found at {}, engine.state:{:?}",
            purge_end - 1,
            st
        );

        log_id.to_log_id()
    }
}
