//! This mod defines external command sent by application to Raft.

use std::fmt;

use crate::core::raft_msg::ResultSender;
use crate::RaftTypeConfig;
use crate::Snapshot;

/// Application-triggered Raft actions for testing and administration.
///
/// Typically, openraft handles actions automatically.
///
/// An application can also disable these policy-based triggering and use these commands manually,
/// for testing or administrative purpose.
pub(crate) enum ExternalCommand<C: RaftTypeConfig> {
    /// Initiate an election at once.
    Elect,

    /// Send a heartbeat message, only if the node is leader, or it will be ignored.
    Heartbeat,

    /// Initiate to build a snapshot on this node.
    Snapshot,

    /// Get a snapshot from the state machine, send back via a oneshot::Sender.
    GetSnapshot { tx: ResultSender<C, Option<Snapshot<C>>> },

    /// Purge logs covered by a snapshot up to a specified index.
    ///
    /// Openraft respects the [`max_in_snapshot_log_to_keep`] config when purging.
    ///
    /// [`max_in_snapshot_log_to_keep`]: `crate::Config::max_in_snapshot_log_to_keep`
    PurgeLog { upto: u64 },
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
        }
    }
}
