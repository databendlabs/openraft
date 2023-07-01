//! This mod defines external command sent by application to Raft.

use std::fmt;

/// Application-triggered Raft actions for testing and administration.
///
/// Typically, openraft handles actions automatically.
///
/// An application can also disable these policy-based triggering and use these commands manually,
/// for testing or administrative purpose.
#[derive(Debug, Clone)]
pub(crate) enum ExternalCommand {
    /// Initiate an election at once.
    Elect,

    /// Send a heartbeat message, only if the node is leader, or it will be ignored.
    Heartbeat,

    /// Initiate to build a snapshot on this node.
    Snapshot,

    /// Purge logs covered by a snapshot up to a specified index.
    ///
    /// Openraft respects the [`max_in_snapshot_log_to_keep`] config when purging.
    ///
    /// [`max_in_snapshot_log_to_keep`]: `crate::Config::max_in_snapshot_log_to_keep`
    PurgeLog { upto: u64 },
}

impl fmt::Display for ExternalCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExternalCommand::Elect => {
                write!(f, "{:?}", self)
            }
            ExternalCommand::Heartbeat => {
                write!(f, "{:?}", self)
            }
            ExternalCommand::Snapshot => {
                write!(f, "{:?}", self)
            }
            ExternalCommand::PurgeLog { upto } => {
                write!(f, "PurgeLog[..={}]", upto)
            }
        }
    }
}
