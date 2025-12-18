/// Enum representing the name of each `ExternalCommand` variant.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExternalCommandName {
    Elect,
    Heartbeat,
    Snapshot,
    GetSnapshot,
    PurgeLog,
    TriggerTransferLeader,
    AllowNextRevert,
    StateMachineCommand,
}

impl ExternalCommandName {
    /// All variants in canonical order.
    #[allow(dead_code)]
    pub const ALL: &'static [ExternalCommandName] = &[
        ExternalCommandName::Elect,
        ExternalCommandName::Heartbeat,
        ExternalCommandName::Snapshot,
        ExternalCommandName::GetSnapshot,
        ExternalCommandName::PurgeLog,
        ExternalCommandName::TriggerTransferLeader,
        ExternalCommandName::AllowNextRevert,
        ExternalCommandName::StateMachineCommand,
    ];

    #[allow(dead_code)]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ExternalCommandName::Elect => "Ext::Elect",
            ExternalCommandName::Heartbeat => "Ext::Heartbeat",
            ExternalCommandName::Snapshot => "Ext::Snapshot",
            ExternalCommandName::GetSnapshot => "Ext::GetSnapshot",
            ExternalCommandName::PurgeLog => "Ext::PurgeLog",
            ExternalCommandName::TriggerTransferLeader => "Ext::TriggerTransferLeader",
            ExternalCommandName::AllowNextRevert => "Ext::AllowNextRevert",
            ExternalCommandName::StateMachineCommand => "Ext::StateMachineCommand",
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
    InstallFullSnapshot,
    BeginReceivingSnapshot,
    ClientWriteRequest,
    EnsureLinearizableRead,
    Initialize,
    ChangeMembership,
    HandleTransferLeader,
    ExternalCoreRequest,
    ExternalCommand(ExternalCommandName),
}

impl RaftMsgName {
    /// All variants in canonical order.
    ///
    /// ExternalCommand variants are expanded to include all ExternalCommandName variants.
    pub const ALL: &'static [RaftMsgName] = &[
        RaftMsgName::AppendEntries,
        RaftMsgName::RequestVote,
        RaftMsgName::InstallFullSnapshot,
        RaftMsgName::BeginReceivingSnapshot,
        RaftMsgName::ClientWriteRequest,
        RaftMsgName::EnsureLinearizableRead,
        RaftMsgName::Initialize,
        RaftMsgName::ChangeMembership,
        RaftMsgName::HandleTransferLeader,
        RaftMsgName::ExternalCoreRequest,
        RaftMsgName::ExternalCommand(ExternalCommandName::Elect),
        RaftMsgName::ExternalCommand(ExternalCommandName::Heartbeat),
        RaftMsgName::ExternalCommand(ExternalCommandName::Snapshot),
        RaftMsgName::ExternalCommand(ExternalCommandName::GetSnapshot),
        RaftMsgName::ExternalCommand(ExternalCommandName::PurgeLog),
        RaftMsgName::ExternalCommand(ExternalCommandName::TriggerTransferLeader),
        RaftMsgName::ExternalCommand(ExternalCommandName::AllowNextRevert),
        RaftMsgName::ExternalCommand(ExternalCommandName::StateMachineCommand),
    ];

    /// Returns the string representation of the message name.
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            RaftMsgName::AppendEntries => "AppendEntries",
            RaftMsgName::RequestVote => "RequestVote",
            RaftMsgName::InstallFullSnapshot => "InstallFullSnapshot",
            RaftMsgName::BeginReceivingSnapshot => "BeginReceivingSnapshot",
            RaftMsgName::ClientWriteRequest => "ClientWriteRequest",
            RaftMsgName::EnsureLinearizableRead => "EnsureLinearizableRead",
            RaftMsgName::Initialize => "Initialize",
            RaftMsgName::ChangeMembership => "ChangeMembership",
            RaftMsgName::HandleTransferLeader => "HandleTransferLeader",
            RaftMsgName::ExternalCoreRequest => "ExternalCoreRequest",
            RaftMsgName::ExternalCommand(ext) => ext.as_str(),
        }
    }
}

impl std::fmt::Display for RaftMsgName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
