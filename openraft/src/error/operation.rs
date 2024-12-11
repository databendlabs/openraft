use std::fmt;

/// API level operation that can be executed on a Raft node.
///
/// These commands represent operations that affect the Raft node's behavior or state. They are
/// primarily used in error reporting to provide context about what operation was attempted when an
/// error occurred.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum Operation {
    /// An unknown operation.
    None,

    /// Set a flag to allow a target replication state to revert to a previous state for one time.
    AllowNextRevert,

    /// Transfer leadership to the specified node.
    TransferLeader,

    /// Send a heartbeat message to a follower or learner.
    SendHeartbeat,

    /// Receive a snapshot.
    ReceiveSnapshot,

    /// Install a snapshot.
    InstallSnapshot,

    /// Write application data via Raft protocol.
    ClientWrite,

    /// Initialize an empty Raft node with a cluster membership config.
    Initialize,

    /// Start an election.
    Elect,
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operation::None => write!(f, "(unknown operation)"),
            Operation::AllowNextRevert => write!(f, "set flag to allow replication revert for once"),
            Operation::TransferLeader => write!(f, "transfer leadership"),
            Operation::SendHeartbeat => write!(f, "send heartbeat"),
            Operation::ReceiveSnapshot => write!(f, "receive snapshot"),
            Operation::InstallSnapshot => write!(f, "install snapshot"),
            Operation::ClientWrite => write!(f, "write application data"),
            Operation::Initialize => write!(f, "initialize"),
            Operation::Elect => write!(f, "elect"),
        }
    }
}
