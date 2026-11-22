//! This mod defines external command sent by application to Raft.

use std::fmt;
use std::sync::Arc;

use display_more::DisplayOptionExt;
use display_more::DisplaySliceExt;

use crate::RaftTypeConfig;
use crate::batch::Batch;
use crate::core::raft_msg::ExternalCommandName;
use crate::core::raft_msg::ResultSender;
use crate::core::raft_msg::leader_removal::LeaderRemoval;
use crate::errors::AllowNextRevertError;
use crate::metrics::MetricsRecorder;
use crate::type_config::alias::BatchOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// Application-triggered Raft actions for testing and administration.
///
/// Typically, openraft handles actions automatically.
///
/// An application can also disable these policy-based triggering and use these commands manually,
/// for testing or administrative purposes.
pub(crate) enum ExternalCommand<C>
where C: RaftTypeConfig
{
    /// Initiate an election, unless this node is a Leader or cannot campaign.
    ///
    /// A Leader ignores it, and so does a node that is not a campaigner in its effective or
    /// committed membership config.
    ///
    /// With `pre_vote = false`, a real election starts at once: the term is incremented and the
    /// node votes for itself, bypassing both `enable_elect` and Pre-Vote. The term climbs even
    /// when the node cannot win, which can step down a healthy leader of a lower term once it
    /// observes the higher term.
    ///
    /// With `pre_vote = true`, a Pre-Vote round first asks whether a quorum *would* grant a vote
    /// at `term + 1`, and the real election starts only once a quorum grants it. The round neither
    /// increments the term nor saves a vote, so a node that cannot win leaves its term untouched,
    /// and an incautious trigger does not disrupt a live leader. The round is skipped while this
    /// node still sees an unexpired leader lease. A rejection that carries a higher vote updates
    /// this node's vote to it, as any higher vote does, and one with a longer log delays the next
    /// election.
    Elect { pre_vote: bool },

    /// Send a heartbeat message, only if the node is leader, or it will be ignored.
    Heartbeat,

    /// Initiate to build a snapshot on this node.
    Snapshot,

    /// Purge logs covered by a snapshot up to and including the log at index `upto`.
    ///
    /// Unlike the purge that follows a snapshot, this ignores the [`max_in_snapshot_log_to_keep`]
    /// config. It purges no log after the last log in the current snapshot, and does nothing
    /// without a snapshot or when an earlier purge already covers `upto`. On a leader, a log that a
    /// replication task still reads is purged only once the task is done with it, see
    /// [`Trigger::purge_log`].
    ///
    /// [`max_in_snapshot_log_to_keep`]: `crate::Config::max_in_snapshot_log_to_keep`
    /// [`Trigger::purge_log`]: crate::raft::trigger::Trigger::purge_log
    PurgeLog { upto: u64 },

    /// Submit a command to inform RaftCore to transfer leadership to the specified node.
    ///
    /// `removal` is `None` for a transfer that an application requests with
    /// [`Trigger::transfer_leader`]. The [`StepDownWatcher`] sets it to the removal of this Leader
    /// that it observed, and `RaftCore` drops the command unless the removal still holds when it
    /// handles the command, see [`Engine::is_removed_leader()`]: a membership change or a new vote
    /// handled meanwhile, such as a readmission of this node, makes the removal stale. A dropped
    /// command clears no leader lease, resets no backoff, and starts no transfer.
    ///
    /// [`Trigger::transfer_leader`]: crate::raft::trigger::Trigger::transfer_leader
    /// [`StepDownWatcher`]: crate::core::StepDownWatcher
    /// [`Engine::is_removed_leader()`]: crate::engine::Engine::is_removed_leader
    TriggerTransferLeader {
        to: C::NodeId,
        removal: Option<LeaderRemoval<C>>,
    },

    /// Reset the backoff state of the replication to the specified nodes.
    ResetBackoff { to: BatchOf<C, C::NodeId> },

    /// Allow or not the next revert of the replication to the specified node.
    AllowNextRevert {
        to: C::NodeId,
        allow: bool,
        tx: ResultSender<C, (), AllowNextRevertError<C>>,
    },

    /// Set or unset a custom metrics recorder for exporting metrics.
    ///
    /// This allows applications to plug in their own metrics collection backends
    /// (e.g., OpenTelemetry, Prometheus, StatsD) at runtime.
    /// Pass `None` to disable metrics recording.
    SetMetricsRecorder { recorder: Option<Arc<dyn MetricsRecorder>> },

    /// Recalculate the internal server state based on the vote and the membership config.
    ///
    /// Most of the time the internal server state is recalculated automatically; the only
    /// exception is the step down of a Leader that is removed from the membership config.
    ///
    /// The command may carry the vote and the effective membership config log id observed by
    /// the sender; it is dropped if either differs from the current state when it is handled,
    /// so that a delayed command can not cause an unexpected refresh. The condition to refresh,
    /// e.g., the membership config that removes the Leader being committed, is checked by the
    /// sender, such as [`StepDownWatcher`].
    ///
    /// A `None` skips the corresponding check: with both `None` the server state is refreshed
    /// unconditionally.
    ///
    /// [`StepDownWatcher`]: crate::core::StepDownWatcher
    RefreshServerState {
        vote: Option<VoteOf<C>>,
        membership_log_id: Option<LogIdOf<C>>,
    },

    /// Check the deadlines that a manual tick checks: on a leader, send a heartbeat if one is due,
    /// then start an election if one is due.
    ///
    /// The heartbeat check is the one that the internal [`Notification::Tick`] runs. Internal
    /// elections do not need a tick: they start on their own deadline notification,
    /// [`Notification::ElectionDeadlineReached`]. The election check is kept here for backward
    /// compatibility, because [`Trigger::tick`] has always started a due election within the same
    /// call.
    ///
    /// [`Notification::Tick`]: crate::core::notification::Notification::Tick
    /// [`Notification::ElectionDeadlineReached`]: crate::core::notification::Notification::ElectionDeadlineReached
    /// [`Trigger::tick`]: crate::raft::trigger::Trigger::tick
    Tick,
}

impl<C> ExternalCommand<C>
where C: RaftTypeConfig
{
    /// Returns the name of this command variant.
    pub fn name(&self) -> ExternalCommandName {
        match self {
            ExternalCommand::Elect { .. } => ExternalCommandName::Elect,
            ExternalCommand::Heartbeat => ExternalCommandName::Heartbeat,
            ExternalCommand::Snapshot => ExternalCommandName::Snapshot,
            ExternalCommand::PurgeLog { .. } => ExternalCommandName::PurgeLog,
            ExternalCommand::TriggerTransferLeader { .. } => ExternalCommandName::TriggerTransferLeader,
            ExternalCommand::ResetBackoff { .. } => ExternalCommandName::ResetBackoff,
            ExternalCommand::AllowNextRevert { .. } => ExternalCommandName::AllowNextRevert,
            ExternalCommand::SetMetricsRecorder { .. } => ExternalCommandName::SetMetricsRecorder,
            ExternalCommand::RefreshServerState { .. } => ExternalCommandName::RefreshServerState,
            ExternalCommand::Tick => ExternalCommandName::Tick,
        }
    }
}

impl<C> fmt::Debug for ExternalCommand<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<C> fmt::Display for ExternalCommand<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExternalCommand::Elect { pre_vote } => {
                write!(f, "Elect{{pre_vote={pre_vote}}}")
            }
            ExternalCommand::Heartbeat => {
                write!(f, "Heartbeat")
            }
            ExternalCommand::Snapshot => {
                write!(f, "Snapshot")
            }
            ExternalCommand::PurgeLog { upto } => {
                write!(f, "PurgeLog[..={}]", upto)
            }
            ExternalCommand::TriggerTransferLeader { to, removal } => {
                write!(f, "TriggerTransferLeader: to {}", to)?;
                if let Some(removal) = removal {
                    write!(f, ", if {} holds", removal)?;
                }
                Ok(())
            }
            ExternalCommand::ResetBackoff { to } => {
                if to.is_empty() {
                    write!(f, "ResetBackoff: to all")
                } else {
                    write!(f, "ResetBackoff: to {}", to.as_ref().display())
                }
            }
            ExternalCommand::AllowNextRevert { to, allow, .. } => {
                // Print the requested flag only: whether a later revert panics also depends on
                // `Config::allow_log_reversion`, which permits every revert when it is enabled.
                write!(f, "AllowNextRevert: to {}, allow: {}", to, allow)
            }
            ExternalCommand::SetMetricsRecorder { .. } => {
                write!(f, "SetMetricsRecorder")
            }
            ExternalCommand::RefreshServerState {
                vote,
                membership_log_id,
            } => {
                write!(
                    f,
                    "RefreshServerState: expected vote: {}, expected membership_log_id: {}",
                    vote.display(),
                    membership_log_id.display()
                )
            }
            ExternalCommand::Tick => {
                write!(f, "Tick")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::core::raft_msg::external_command::ExternalCommand;
    use crate::engine::testing::UTConfig;
    use crate::type_config::TypeConfigExt;

    /// The display names the requested flag, and predicts nothing about the next revert.
    #[test]
    fn test_display_allow_next_revert() {
        for allow in [true, false] {
            let (tx, _rx) = UTConfig::<()>::oneshot();
            let cmd = ExternalCommand::<UTConfig>::AllowNextRevert { to: 1, allow, tx };
            assert_eq!(format!("AllowNextRevert: to 1, allow: {allow}"), cmd.to_string());
        }
    }
}
