//! This mod defines external command sent by application to Raft.

use std::fmt;

use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::core::raft_msg::ResultSender;
use crate::core::sm;
use crate::error::AllowNextRevertError;
use crate::type_config::alias::OneshotSenderOf;

/// Application-triggered Raft actions for testing and administration.
///
/// Typically, openraft handles actions automatically.
///
/// An application can also disable these policy-based triggering and use these commands manually,
/// for testing or administrative purposes.
pub(crate) enum ExternalCommand<C: RaftTypeConfig> {
    /// Initiate an election at once.
    Elect,

    /// Send a heartbeat message, only if the node is leader, or it will be ignored.
    Heartbeat,

    /// Initiate to build a snapshot on this node.
    Snapshot,

    /// Get a snapshot from the state machine, send back via a oneshot::Sender.
    GetSnapshot {
        tx: OneshotSenderOf<C, Option<Snapshot<C>>>,
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

    /// Send a [`sm::Command`] to [`sm::worker::Worker`].
    /// This command is run in the sm task.
    StateMachineCommand { sm_cmd: sm::Command<C> },
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
            ExternalCommand::Elect => {
                write!(f, "Elect")
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
            ExternalCommand::StateMachineCommand { sm_cmd } => {
                write!(f, "StateMachineCommand: {}", sm_cmd)
            }
        }
    }
}
