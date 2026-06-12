//! This mod defines external command sent by application to Raft.

use std::fmt;
use std::sync::Arc;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::core::raft_msg::ExternalCommandName;
use crate::core::raft_msg::ResultSender;
use crate::errors::AllowNextRevertError;
use crate::metrics::MetricsRecorder;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::VoteOf;

/// Application-triggered Raft actions for testing and administration.
///
/// Typically, openraft handles actions automatically.
///
/// An application can also disable these policy-based triggering and use these commands manually,
/// for testing or administrative purposes.
pub(crate) enum ExternalCommand<C: RaftTypeConfig> {
    /// Initiate an election immediately.
    ///
    /// With `pre_vote = false`, a real election starts at once: the term is incremented and the
    /// node votes for itself, bypassing both `enable_elect` and Pre-Vote. The term climbs even
    /// when the node cannot win, which can step down a healthy leader of a lower term once it
    /// observes the higher term.
    ///
    /// With `pre_vote = true`, a Pre-Vote round runs first: it probes whether a quorum *would*
    /// grant a vote at `term + 1` without changing any state, and runs the real election only
    /// if a quorum would. A node that cannot currently win — e.g. while a healthy leader still
    /// holds its lease — leaves the term untouched, so an incautious trigger does not disrupt a
    /// live leader.
    Elect { pre_vote: bool },

    /// Send a heartbeat message, only if the node is leader, or it will be ignored.
    Heartbeat,

    /// Initiate to build a snapshot on this node.
    Snapshot,

    /// Get a snapshot from the state machine, send back via a oneshot::Sender.
    GetSnapshot {
        tx: OneshotSenderOf<C, Option<SnapshotOf<C>>>,
    },

    /// Purge logs covered by a snapshot up to a specified index.
    ///
    /// Openraft respects the [`max_in_snapshot_log_to_keep`] config when purging.
    ///
    /// [`max_in_snapshot_log_to_keep`]: `crate::Config::max_in_snapshot_log_to_keep`
    PurgeLog { upto: u64 },

    /// Submit a command to inform RaftCore to transfer leadership to the specified node.
    TriggerTransferLeader { to: C::NodeId },

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
}

impl<C: RaftTypeConfig> ExternalCommand<C> {
    /// Returns the name of this command variant.
    pub fn name(&self) -> ExternalCommandName {
        match self {
            ExternalCommand::Elect { .. } => ExternalCommandName::Elect,
            ExternalCommand::Heartbeat => ExternalCommandName::Heartbeat,
            ExternalCommand::Snapshot => ExternalCommandName::Snapshot,
            ExternalCommand::GetSnapshot { .. } => ExternalCommandName::GetSnapshot,
            ExternalCommand::PurgeLog { .. } => ExternalCommandName::PurgeLog,
            ExternalCommand::TriggerTransferLeader { .. } => ExternalCommandName::TriggerTransferLeader,
            ExternalCommand::AllowNextRevert { .. } => ExternalCommandName::AllowNextRevert,
            ExternalCommand::SetMetricsRecorder { .. } => ExternalCommandName::SetMetricsRecorder,
            ExternalCommand::RefreshServerState { .. } => ExternalCommandName::RefreshServerState,
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
            ExternalCommand::GetSnapshot { .. } => {
                write!(f, "GetSnapshot")
            }
            ExternalCommand::PurgeLog { upto } => {
                write!(f, "PurgeLog[..={}]", upto)
            }
            ExternalCommand::TriggerTransferLeader { to } => {
                write!(f, "TriggerTransferLeader: to {}", to)
            }
            ExternalCommand::AllowNextRevert { to, allow, .. } => {
                write!(
                    f,
                    "{}-on-next-log-revert: to {}",
                    if *allow { "AllowReset" } else { "Panic" },
                    to
                )
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
        }
    }
}
