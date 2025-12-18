/// Enum representing the name of each `sm::Command` variant.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub(crate) enum SMCommandName {
    BuildSnapshot = 0,
    GetSnapshot = 1,
    BeginReceivingSnapshot = 2,
    InstallFullSnapshot = 3,
    Apply = 4,
    Func = 5,
}

impl SMCommandName {
    /// All variants in canonical order.
    #[allow(dead_code)]
    pub const ALL: &'static [SMCommandName] = &[
        SMCommandName::BuildSnapshot,
        SMCommandName::GetSnapshot,
        SMCommandName::BeginReceivingSnapshot,
        SMCommandName::InstallFullSnapshot,
        SMCommandName::Apply,
        SMCommandName::Func,
    ];

    #[allow(dead_code)]
    pub const fn as_str(&self) -> &'static str {
        match self {
            SMCommandName::BuildSnapshot => "SM::BuildSnapshot",
            SMCommandName::GetSnapshot => "SM::GetSnapshot",
            SMCommandName::BeginReceivingSnapshot => "SM::BeginReceivingSnapshot",
            SMCommandName::InstallFullSnapshot => "SM::InstallFullSnapshot",
            SMCommandName::Apply => "SM::Apply",
            SMCommandName::Func => "SM::Func",
        }
    }
}

impl std::fmt::Display for SMCommandName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Enum representing the name of each `Command` variant.
///
/// This provides an efficient way to identify command types without
/// string comparisons, useful for logging, metrics, and debugging.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum CommandName {
    UpdateIOProgress,
    AppendEntries,
    ReplicateCommitted,
    BroadcastHeartbeat,
    SaveCommittedAndApply,
    Replicate,
    ReplicateSnapshot,
    BroadcastTransferLeader,
    CloseReplicationStreams,
    RebuildReplicationStreams,
    SaveVote,
    SendVote,
    PurgeLog,
    TruncateLog,
    StateMachine(SMCommandName),
    Respond,
}

impl CommandName {
    /// All variants in canonical order.
    ///
    /// StateMachine variants are expanded to include all SMCommandName variants.
    pub const ALL: &'static [CommandName] = &[
        CommandName::UpdateIOProgress,
        CommandName::AppendEntries,
        CommandName::ReplicateCommitted,
        CommandName::BroadcastHeartbeat,
        CommandName::SaveCommittedAndApply,
        CommandName::Replicate,
        CommandName::ReplicateSnapshot,
        CommandName::BroadcastTransferLeader,
        CommandName::CloseReplicationStreams,
        CommandName::RebuildReplicationStreams,
        CommandName::SaveVote,
        CommandName::SendVote,
        CommandName::PurgeLog,
        CommandName::TruncateLog,
        CommandName::StateMachine(SMCommandName::BuildSnapshot),
        CommandName::StateMachine(SMCommandName::GetSnapshot),
        CommandName::StateMachine(SMCommandName::BeginReceivingSnapshot),
        CommandName::StateMachine(SMCommandName::InstallFullSnapshot),
        CommandName::StateMachine(SMCommandName::Apply),
        CommandName::StateMachine(SMCommandName::Func),
        CommandName::Respond,
    ];

    /// Returns the string representation of the command name.
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            CommandName::UpdateIOProgress => "UpdateIOProgress",
            CommandName::AppendEntries => "AppendEntries",
            CommandName::ReplicateCommitted => "ReplicateCommitted",
            CommandName::BroadcastHeartbeat => "BroadcastHeartbeat",
            CommandName::SaveCommittedAndApply => "SaveCommittedAndApply",
            CommandName::Replicate => "Replicate",
            CommandName::ReplicateSnapshot => "ReplicateSnapshot",
            CommandName::BroadcastTransferLeader => "BroadcastTransferLeader",
            CommandName::CloseReplicationStreams => "CloseReplicationStreams",
            CommandName::RebuildReplicationStreams => "RebuildReplicationStreams",
            CommandName::SaveVote => "SaveVote",
            CommandName::SendVote => "SendVote",
            CommandName::PurgeLog => "PurgeLog",
            CommandName::TruncateLog => "TruncateLog",
            CommandName::StateMachine(sm) => sm.as_str(),
            CommandName::Respond => "Respond",
        }
    }
}

impl std::fmt::Display for CommandName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::CommandName;
    use super::SMCommandName;
    use crate::core::sm;
    use crate::engine::Command;
    use crate::engine::TargetProgress;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::impls::Vote;
    use crate::progress::entry::ProgressEntry;
    use crate::progress::inflight_id::InflightId;
    use crate::progress::stream_id::StreamId;
    use crate::raft::VoteRequest;
    use crate::raft::message::TransferLeaderRequest;
    use crate::raft_state::IOId;
    use crate::replication::ReplicationSessionId;
    use crate::replication::replicate::Replicate;
    use crate::vote::committed::CommittedVote;
    use crate::vote::raft_vote::RaftVoteExt;

    type C = UTConfig;

    fn committed_vote(term: u64, node_id: u64) -> CommittedVote<C> {
        Vote::<C>::new(term, node_id).into_committed()
    }

    #[test]
    fn test_command_name() {
        let cv = committed_vote(1, 0);

        // UpdateIOProgress
        let cmd: Command<C> = Command::UpdateIOProgress {
            when: None,
            io_id: IOId::new_log_io(cv.clone(), Some(log_id(1, 0, 1))),
        };
        assert_eq!(cmd.name(), CommandName::UpdateIOProgress);

        // AppendEntries
        let cmd: Command<C> = Command::AppendEntries {
            committed_vote: cv.clone(),
            entries: vec![],
        };
        assert_eq!(cmd.name(), CommandName::AppendEntries);

        // ReplicateCommitted
        let cmd: Command<C> = Command::ReplicateCommitted { committed: None };
        assert_eq!(cmd.name(), CommandName::ReplicateCommitted);

        // BroadcastHeartbeat
        let cmd: Command<C> = Command::BroadcastHeartbeat {
            session_id: ReplicationSessionId::new(cv.clone(), None),
        };
        assert_eq!(cmd.name(), CommandName::BroadcastHeartbeat);

        // SaveCommittedAndApply
        let cmd: Command<C> = Command::SaveCommittedAndApply {
            already_applied: None,
            upto: log_id(1, 0, 1),
        };
        assert_eq!(cmd.name(), CommandName::SaveCommittedAndApply);

        // Replicate
        let cmd: Command<C> = Command::Replicate {
            target: 1,
            req: Replicate::new_logs(
                crate::log_id_range::LogIdRange::new(None, Some(log_id(1, 0, 1))),
                InflightId::new(1),
            ),
        };
        assert_eq!(cmd.name(), CommandName::Replicate);

        // ReplicateSnapshot
        let cmd: Command<C> = Command::ReplicateSnapshot {
            leader_vote: cv.clone(),
            target: 1,
            inflight_id: InflightId::new(1),
        };
        assert_eq!(cmd.name(), CommandName::ReplicateSnapshot);

        // BroadcastTransferLeader
        let cmd: Command<C> = Command::BroadcastTransferLeader {
            req: TransferLeaderRequest::new(Vote::new(1, 0), 1, None),
        };
        assert_eq!(cmd.name(), CommandName::BroadcastTransferLeader);

        // CloseReplicationStreams
        let cmd: Command<C> = Command::CloseReplicationStreams;
        assert_eq!(cmd.name(), CommandName::CloseReplicationStreams);

        // RebuildReplicationStreams
        let cmd: Command<C> = Command::RebuildReplicationStreams {
            leader_vote: cv.clone(),
            targets: vec![TargetProgress {
                target: 1,
                target_node: (),
                progress: ProgressEntry::empty(StreamId::new(1), 1),
            }],
            close_old_streams: false,
        };
        assert_eq!(cmd.name(), CommandName::RebuildReplicationStreams);

        // SaveVote
        let cmd: Command<C> = Command::SaveVote { vote: Vote::new(1, 0) };
        assert_eq!(cmd.name(), CommandName::SaveVote);

        // SendVote
        let cmd: Command<C> = Command::SendVote {
            vote_req: VoteRequest::new(Vote::new(1, 0), None),
        };
        assert_eq!(cmd.name(), CommandName::SendVote);

        // PurgeLog
        let cmd: Command<C> = Command::PurgeLog { upto: log_id(1, 0, 1) };
        assert_eq!(cmd.name(), CommandName::PurgeLog);

        // TruncateLog
        let cmd: Command<C> = Command::TruncateLog { since: log_id(1, 0, 1) };
        assert_eq!(cmd.name(), CommandName::TruncateLog);

        // Respond - skip as it requires a oneshot sender
    }

    #[test]
    fn test_sm_command_name() {
        // BuildSnapshot
        let cmd: sm::Command<C> = sm::Command::build_snapshot();
        assert_eq!(cmd.name(), SMCommandName::BuildSnapshot);

        // Apply
        let cmd: sm::Command<C> = sm::Command::apply(log_id(1, 0, 1), log_id(1, 0, 2), vec![]);
        assert_eq!(cmd.name(), SMCommandName::Apply);

        // GetSnapshot, BeginReceivingSnapshot, InstallFullSnapshot require channels/data
        // Test via StateMachine command wrapper
        let cmd: Command<C> = Command::StateMachine {
            command: sm::Command::build_snapshot(),
        };
        assert_eq!(cmd.name(), CommandName::StateMachine(SMCommandName::BuildSnapshot));

        let cmd: Command<C> = Command::StateMachine {
            command: sm::Command::apply(log_id(1, 0, 1), log_id(1, 0, 2), vec![]),
        };
        assert_eq!(cmd.name(), CommandName::StateMachine(SMCommandName::Apply));
    }

    #[test]
    fn test_command_name_as_str() {
        assert_eq!(CommandName::UpdateIOProgress.as_str(), "UpdateIOProgress");
        assert_eq!(CommandName::AppendEntries.as_str(), "AppendEntries");
        assert_eq!(CommandName::ReplicateCommitted.as_str(), "ReplicateCommitted");
        assert_eq!(CommandName::BroadcastHeartbeat.as_str(), "BroadcastHeartbeat");
        assert_eq!(CommandName::SaveCommittedAndApply.as_str(), "SaveCommittedAndApply");
        assert_eq!(CommandName::Replicate.as_str(), "Replicate");
        assert_eq!(CommandName::ReplicateSnapshot.as_str(), "ReplicateSnapshot");
        assert_eq!(CommandName::BroadcastTransferLeader.as_str(), "BroadcastTransferLeader");
        assert_eq!(CommandName::CloseReplicationStreams.as_str(), "CloseReplicationStreams");
        assert_eq!(
            CommandName::RebuildReplicationStreams.as_str(),
            "RebuildReplicationStreams"
        );
        assert_eq!(CommandName::SaveVote.as_str(), "SaveVote");
        assert_eq!(CommandName::SendVote.as_str(), "SendVote");
        assert_eq!(CommandName::PurgeLog.as_str(), "PurgeLog");
        assert_eq!(CommandName::TruncateLog.as_str(), "TruncateLog");
        assert_eq!(CommandName::Respond.as_str(), "Respond");

        // StateMachine delegates to SMCommandName
        assert_eq!(
            CommandName::StateMachine(SMCommandName::BuildSnapshot).as_str(),
            "SM::BuildSnapshot"
        );
        assert_eq!(CommandName::StateMachine(SMCommandName::Apply).as_str(), "SM::Apply");
    }

    #[test]
    fn test_sm_command_name_as_str() {
        assert_eq!(SMCommandName::BuildSnapshot.as_str(), "SM::BuildSnapshot");
        assert_eq!(SMCommandName::GetSnapshot.as_str(), "SM::GetSnapshot");
        assert_eq!(
            SMCommandName::BeginReceivingSnapshot.as_str(),
            "SM::BeginReceivingSnapshot"
        );
        assert_eq!(SMCommandName::InstallFullSnapshot.as_str(), "SM::InstallFullSnapshot");
        assert_eq!(SMCommandName::Apply.as_str(), "SM::Apply");
        assert_eq!(SMCommandName::Func.as_str(), "SM::Func");
    }
}
