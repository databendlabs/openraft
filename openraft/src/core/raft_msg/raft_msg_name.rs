use openraft_macros::VariantName;
use openraft_macros::since;

/// Enum representing the name of each `ExternalCommand` variant.
#[since(version = "0.10.1", change = "added Tick and RefreshTickDeadline variants")]
#[since(version = "0.10.0", change = "added ResetBackoff variant")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(VariantName)]
#[variant_name(prefix = "Ext::")]
pub enum ExternalCommandName {
    /// Start an election.
    Elect,
    /// Send a heartbeat if this node is the leader.
    Heartbeat,
    /// Build a snapshot.
    Snapshot,
    /// Purge the logs that a snapshot covers, up to an index.
    PurgeLog,
    /// Transfer leadership to a given node.
    TriggerTransferLeader,
    /// Reset the replication backoff for given nodes.
    ResetBackoff,
    /// Allow or forbid the next log reversion of the replication to a node.
    AllowNextRevert,
    /// Set or unset a custom metrics recorder.
    SetMetricsRecorder,
    /// Recompute the server state from the vote and the membership.
    RefreshServerState,
    /// Run one timer check (election timeout and leader heartbeat).
    #[since(version = "0.10.1")]
    Tick,
    /// Republish the shared tick deadline without running a timer check.
    #[since(version = "0.10.1")]
    RefreshTickDeadline,
}

/// Enum naming each Raft message type tracked for logging, metrics, and debugging.
///
/// Most variants correspond one-to-one with a `RaftMsg` variant. The exception is
/// `InstallSnapshot`: full-snapshot installation is delivered through a dedicated channel rather
/// than as a `RaftMsg` variant, but is still recorded here for runtime stats.
///
/// This provides an efficient way to identify message types without string comparisons.
#[since]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(VariantName)]
pub enum RaftMsgName {
    /// An AppendEntries request.
    AppendEntries,
    /// A vote request.
    RequestVote,
    /// A Pre-Vote request.
    RequestPreVote,
    /// A full snapshot to install.
    InstallSnapshot,
    /// A client write request.
    ClientWrite,
    /// A request for a read linearizer.
    GetLinearizer,
    /// A request to initialize the cluster.
    Initialize,
    /// A membership change request.
    ChangeMembership,
    /// A request to append a caller-built membership.
    #[since(version = "0.10.0")]
    AppendMembership,
    /// A leadership transfer request received from the leader.
    HandleTransferLeader,
    /// A function to run on the Raft state.
    WithRaftState,
    /// An external command, named by the inner value.
    ExternalCommand(ExternalCommandName),
    /// A request for runtime statistics.
    GetRuntimeStats,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_external_command_name_index() {
        assert_eq!(ExternalCommandName::COUNT, ExternalCommandName::ALL.len());

        for (i, name) in ExternalCommandName::ALL.iter().enumerate() {
            assert_eq!(
                name.index(),
                i,
                "ExternalCommandName::{:?} index mismatch: expected {}, got {}",
                name,
                i,
                name.index()
            );
        }
    }

    #[test]
    fn test_raft_msg_name_index() {
        assert_eq!(RaftMsgName::COUNT, RaftMsgName::ALL.len());

        for (i, name) in RaftMsgName::ALL.iter().enumerate() {
            assert_eq!(
                name.index(),
                i,
                "RaftMsgName::{:?} index mismatch: expected {}, got {}",
                name,
                i,
                name.index()
            );
        }
    }
}
