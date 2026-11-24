use maplit::btreeset;

use super::CommandName;
use super::SMCommandName;
use crate::Membership;
use crate::base::BoxAsyncOnceMut;
use crate::batch::Batch;
use crate::core::sm;
use crate::engine::Command;
use crate::engine::Respond;
use crate::engine::TargetProgress;
use crate::engine::testing::UTConfig;
use crate::engine::testing::UTLeaderId;
use crate::engine::testing::log_id;
use crate::errors::InitializeError;
use crate::impls::Vote;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::raft::VoteRequest;
use crate::raft::message::TransferLeaderRequest;
use crate::raft_state::IOId;
use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::replication::ReplicationSessionId;
use crate::replication::replicate::Replicate;
use crate::storage::Snapshot;
use crate::storage::SnapshotMeta;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::vote::committed::CommittedVote;
use crate::vote::raft_vote::RaftVoteExt;

type C = UTConfig;

fn committed_vote(term: u64, node_id: u64) -> CommittedVote<UTLeaderId> {
    Vote::<UTLeaderId>::new(term, node_id).to_committed()
}

/// One command of each worker variant, with the name it must report.
fn sm_commands() -> Vec<(sm::Command<C>, SMCommandName)> {
    let (snapshot_tx, _snapshot_rx) = C::oneshot();
    let meta = SnapshotMeta {
        last_log_id: Some(log_id(1, 0, 3)),
        last_membership: StoredMembershipOf::<C>::new(
            Some(log_id(1, 0, 1)),
            Membership::new_with_defaults(vec![btreeset! {0}], []),
        ),
    };
    let snapshot = Snapshot::new(meta, ());
    let log_io_id = LogIOId::new(committed_vote(1, 0), Some(log_id(1, 0, 3)));
    let func: BoxAsyncOnceMut<'static, ()> = Box::new(|_sm: &mut ()| Box::pin(async {}));

    vec![
        (sm::Command::build_snapshot(), SMCommandName::BuildSnapshot),
        (sm::Command::get_snapshot(snapshot_tx), SMCommandName::GetSnapshot),
        (
            sm::Command::install_full_snapshot(snapshot, log_io_id),
            SMCommandName::InstallFullSnapshot,
        ),
        (
            sm::Command::apply(log_id(1, 0, 1), log_id(1, 0, 2), vec![]),
            SMCommandName::Apply,
        ),
        (sm::Command::ExternalFunc { func }, SMCommandName::ExternalFunc),
    ]
}

/// One command of each engine variant, and of each worker variant inside `StateMachine`, with the
/// name it must report.
fn commands() -> Vec<(Command<C>, CommandName)> {
    let cv = committed_vote(1, 0);
    let (initialize_tx, _initialize_rx) = C::oneshot::<Result<(), InitializeError<C>>>();

    let mut cases: Vec<(Command<C>, CommandName)> = vec![
        (
            Command::UpdateIOProgress {
                when: None,
                io_id: IOId::new_log_io(cv.clone(), Some(log_id(1, 0, 1))),
            },
            CommandName::UpdateIOProgress,
        ),
        (
            Command::AppendEntries {
                committed_vote: cv.clone(),
                entries: Batch::of([]),
            },
            CommandName::AppendEntries,
        ),
        (
            Command::ReplicateCommitted { committed: None },
            CommandName::ReplicateCommitted,
        ),
        (
            Command::BroadcastHeartbeat {
                session_id: ReplicationSessionId::new(cv.clone(), None),
                bypass_min_interval: false,
            },
            CommandName::BroadcastHeartbeat,
        ),
        (
            Command::SaveCommittedAndApply {
                already_applied: None,
                upto: log_id(1, 0, 1),
            },
            CommandName::SaveCommittedAndApply,
        ),
        (
            Command::Replicate {
                target: 1,
                req: Replicate::new_probe(
                    crate::log_id_range::LogIdRange::new(None, Some(log_id(1, 0, 1))),
                    InflightId::new(1),
                ),
            },
            CommandName::Replicate,
        ),
        (
            Command::ClearReplicate {
                target: 1,
                inflight_id: InflightId::new(1),
            },
            CommandName::ClearReplicate,
        ),
        (
            Command::ReplicateSnapshot {
                leader_vote: cv.clone(),
                target: 1,
                inflight_id: InflightId::new(1),
            },
            CommandName::ReplicateSnapshot,
        ),
        (
            Command::BroadcastTransferLeader {
                req: TransferLeaderRequest::new(Vote::new(1, 0), 1, None),
            },
            CommandName::BroadcastTransferLeader,
        ),
        (Command::CloseReplicationStreams, CommandName::CloseReplicationStreams),
        (Command::FailPendingReads, CommandName::FailPendingReads),
        (
            Command::RebuildReplicationStreams {
                leader_vote: cv.clone(),
                targets: vec![TargetProgress {
                    target_node: (),
                    progress: ProgressEntry::empty(1, StreamId::new(1), 1),
                }],
                close_old_streams: false,
            },
            CommandName::RebuildReplicationStreams,
        ),
        (Command::SaveVote { vote: Vote::new(1, 0) }, CommandName::SaveVote),
        (
            Command::SendVote {
                vote_req: VoteRequest::new(Vote::new(1, 0), None),
            },
            CommandName::SendVote,
        ),
        (
            Command::SendPreVote {
                vote_req: VoteRequest::new(Vote::new(1, 0), None),
            },
            CommandName::SendPreVote,
        ),
        (Command::PurgeLog { upto: log_id(1, 0, 1) }, CommandName::PurgeLog),
        (
            Command::TruncateLog {
                after: Some(log_id(1, 0, 1)),
            },
            CommandName::TruncateLog,
        ),
        (
            Command::Respond {
                when: None,
                resp: Respond::new(Ok(()), initialize_tx),
            },
            CommandName::Respond,
        ),
    ];

    for (command, name) in sm_commands() {
        cases.push((Command::StateMachine { command }, CommandName::StateMachine(name)));
    }
    cases
}

/// Sort `names`, and check that they are exactly `all`: one case per name, none repeated.
fn assert_each_name_once<N>(all: &[N], mut names: Vec<N>)
where N: Ord + Copy + std::fmt::Debug {
    names.sort();
    let mut all = all.to_vec();
    all.sort();
    assert_eq!(all, names, "each name has exactly one case");
}

#[test]
fn test_command_name() {
    let cases = commands();
    for (cmd, want) in &cases {
        assert_eq!(*want, cmd.name(), "the name of {}", cmd);
    }
    assert_each_name_once(CommandName::ALL, cases.iter().map(|(_, name)| *name).collect());
}

#[test]
fn test_sm_command_name() {
    let cases = sm_commands();
    for (cmd, want) in &cases {
        assert_eq!(*want, cmd.name(), "the name of {}", cmd);
    }
    assert_each_name_once(SMCommandName::ALL, cases.iter().map(|(_, name)| *name).collect());
}

#[test]
fn test_command_name_as_str() {
    assert_eq!(CommandName::UpdateIOProgress.as_str(), "UpdateIOProgress");
    assert_eq!(CommandName::AppendEntries.as_str(), "AppendEntries");
    assert_eq!(CommandName::ReplicateCommitted.as_str(), "ReplicateCommitted");
    assert_eq!(CommandName::BroadcastHeartbeat.as_str(), "BroadcastHeartbeat");
    assert_eq!(CommandName::SaveCommittedAndApply.as_str(), "SaveCommittedAndApply");
    assert_eq!(CommandName::Replicate.as_str(), "Replicate");
    assert_eq!(CommandName::ClearReplicate.as_str(), "ClearReplicate");
    assert_eq!(CommandName::ReplicateSnapshot.as_str(), "ReplicateSnapshot");
    assert_eq!(CommandName::BroadcastTransferLeader.as_str(), "BroadcastTransferLeader");
    assert_eq!(CommandName::CloseReplicationStreams.as_str(), "CloseReplicationStreams");
    assert_eq!(CommandName::FailPendingReads.as_str(), "FailPendingReads");
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
    assert_eq!(SMCommandName::InstallFullSnapshot.as_str(), "SM::InstallFullSnapshot");
    assert_eq!(SMCommandName::Apply.as_str(), "SM::Apply");
    assert_eq!(SMCommandName::ExternalFunc.as_str(), "SM::ExternalFunc");
}

#[test]
fn test_sm_command_name_index() {
    assert_eq!(SMCommandName::COUNT, SMCommandName::ALL.len());

    for (i, name) in SMCommandName::ALL.iter().enumerate() {
        assert_eq!(
            name.index(),
            i,
            "SMCommandName::{:?} index mismatch: expected {}, got {}",
            name,
            i,
            name.index()
        );
    }
}

#[test]
fn test_command_name_index() {
    assert_eq!(CommandName::COUNT, CommandName::ALL.len());

    for (i, name) in CommandName::ALL.iter().enumerate() {
        assert_eq!(
            name.index(),
            i,
            "CommandName::{:?} index mismatch: expected {}, got {}",
            name,
            i,
            name.index()
        );
    }
}
