use openraft_macros::VariantName;
use openraft_macros::since;

/// Enum representing the name of each `ExternalCommand` variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(VariantName)]
#[variant_name(prefix = "Ext::")]
pub enum ExternalCommandName {
    Elect,
    Heartbeat,
    Snapshot,
    PurgeLog,
    TriggerTransferLeader,
    AllowNextRevert,
    SetMetricsRecorder,
    RefreshServerState,
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
    AppendEntries,
    RequestVote,
    RequestPreVote,
    InstallSnapshot,
    ClientWrite,
    GetLinearizer,
    Initialize,
    ChangeMembership,
    #[since(version = "0.10.0")]
    AppendMembership,
    HandleTransferLeader,
    WithRaftState,
    ExternalCommand(ExternalCommandName),
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
