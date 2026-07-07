/// Enum representing the name of each `ExternalCommand` variant.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

impl ExternalCommandName {
    /// Total number of variants.
    #[allow(dead_code)]
    pub const COUNT: usize = 8;

    /// All variants in canonical order.
    #[allow(dead_code)]
    pub const ALL: &'static [ExternalCommandName] = &[
        ExternalCommandName::Elect,
        ExternalCommandName::Heartbeat,
        ExternalCommandName::Snapshot,
        ExternalCommandName::PurgeLog,
        ExternalCommandName::TriggerTransferLeader,
        ExternalCommandName::AllowNextRevert,
        ExternalCommandName::SetMetricsRecorder,
        ExternalCommandName::RefreshServerState,
    ];

    /// Returns the index of this variant for array-based storage.
    #[allow(dead_code)]
    pub const fn index(&self) -> usize {
        match self {
            ExternalCommandName::Elect => 0,
            ExternalCommandName::Heartbeat => 1,
            ExternalCommandName::Snapshot => 2,
            ExternalCommandName::PurgeLog => 3,
            ExternalCommandName::TriggerTransferLeader => 4,
            ExternalCommandName::AllowNextRevert => 5,
            ExternalCommandName::SetMetricsRecorder => 6,
            ExternalCommandName::RefreshServerState => 7,
        }
    }

    #[allow(dead_code)]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ExternalCommandName::Elect => "Ext::Elect",
            ExternalCommandName::Heartbeat => "Ext::Heartbeat",
            ExternalCommandName::Snapshot => "Ext::Snapshot",
            ExternalCommandName::PurgeLog => "Ext::PurgeLog",
            ExternalCommandName::TriggerTransferLeader => "Ext::TriggerTransferLeader",
            ExternalCommandName::AllowNextRevert => "Ext::AllowNextRevert",
            ExternalCommandName::SetMetricsRecorder => "Ext::SetMetricsRecorder",
            ExternalCommandName::RefreshServerState => "Ext::RefreshServerState",
        }
    }
}

impl std::fmt::Display for ExternalCommandName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Enum representing the name of each `RaftMsg` variant.
///
/// This provides an efficient way to identify message types without
/// string comparisons, useful for logging, metrics, and debugging.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RaftMsgName {
    AppendEntries,
    RequestVote,
    RequestPreVote,
    InstallSnapshot,
    ClientWrite,
    GetLinearizer,
    Initialize,
    ChangeMembership,
    HandleTransferLeader,
    WithRaftState,
    ExternalCommand(ExternalCommandName),
    GetRuntimeStats,
}

impl RaftMsgName {
    /// Total number of variants (including expanded ExternalCommand variants).
    pub const COUNT: usize = 19;

    /// All variants in canonical order.
    ///
    /// ExternalCommand variants are expanded to include all ExternalCommandName variants.
    #[allow(dead_code)]
    pub const ALL: &'static [RaftMsgName] = &[
        RaftMsgName::AppendEntries,
        RaftMsgName::RequestVote,
        RaftMsgName::RequestPreVote,
        RaftMsgName::InstallSnapshot,
        RaftMsgName::ClientWrite,
        RaftMsgName::GetLinearizer,
        RaftMsgName::Initialize,
        RaftMsgName::ChangeMembership,
        RaftMsgName::HandleTransferLeader,
        RaftMsgName::WithRaftState,
        RaftMsgName::ExternalCommand(ExternalCommandName::Elect),
        RaftMsgName::ExternalCommand(ExternalCommandName::Heartbeat),
        RaftMsgName::ExternalCommand(ExternalCommandName::Snapshot),
        RaftMsgName::ExternalCommand(ExternalCommandName::PurgeLog),
        RaftMsgName::ExternalCommand(ExternalCommandName::TriggerTransferLeader),
        RaftMsgName::ExternalCommand(ExternalCommandName::AllowNextRevert),
        RaftMsgName::ExternalCommand(ExternalCommandName::SetMetricsRecorder),
        RaftMsgName::ExternalCommand(ExternalCommandName::RefreshServerState),
        RaftMsgName::GetRuntimeStats,
    ];

    /// Returns the index of this variant for array-based storage.
    pub const fn index(&self) -> usize {
        match self {
            RaftMsgName::AppendEntries => 0,
            RaftMsgName::RequestVote => 1,
            RaftMsgName::RequestPreVote => 2,
            RaftMsgName::InstallSnapshot => 3,
            RaftMsgName::ClientWrite => 4,
            RaftMsgName::GetLinearizer => 5,
            RaftMsgName::Initialize => 6,
            RaftMsgName::ChangeMembership => 7,
            RaftMsgName::HandleTransferLeader => 8,
            RaftMsgName::WithRaftState => 9,
            RaftMsgName::ExternalCommand(ext) => 10 + ext.index(),
            RaftMsgName::GetRuntimeStats => 10 + ExternalCommandName::COUNT,
        }
    }

    /// Returns the string representation of the message name.
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            RaftMsgName::AppendEntries => "AppendEntries",
            RaftMsgName::RequestVote => "RequestVote",
            RaftMsgName::RequestPreVote => "RequestPreVote",
            RaftMsgName::InstallSnapshot => "InstallSnapshot",
            RaftMsgName::ClientWrite => "ClientWrite",
            RaftMsgName::GetLinearizer => "GetLinearizer",
            RaftMsgName::Initialize => "Initialize",
            RaftMsgName::ChangeMembership => "ChangeMembership",
            RaftMsgName::HandleTransferLeader => "HandleTransferLeader",
            RaftMsgName::WithRaftState => "WithRaftState",
            RaftMsgName::ExternalCommand(ext) => ext.as_str(),
            RaftMsgName::GetRuntimeStats => "GetRuntimeStats",
        }
    }
}

impl std::fmt::Display for RaftMsgName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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
