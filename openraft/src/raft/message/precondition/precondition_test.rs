use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use crate::Membership;
use crate::MembershipState;
use crate::RaftState;
use crate::Vote;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::errors::PreconditionFailed;
use crate::raft::Precondition;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::type_config::alias::VoteOf;
use crate::utime::Leased;
use crate::vote::RaftLeaderIdExt;

fn committed_leader_id(term: u64, node_id: u64) -> CommittedLeaderIdOf<UTConfig> {
    LeaderIdOf::<UTConfig>::new_committed(term, node_id)
}

/// Build a state whose last log id is the last of `log_ids`, and whose effective membership is
/// stored at `membership_log_id`.
fn state(
    vote: VoteOf<UTConfig>,
    log_ids: Vec<LogIdOf<UTConfig>>,
    membership_log_id: Option<LogIdOf<UTConfig>>,
) -> RaftState<UTConfig> {
    let membership = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2}], []);
    let stored = Arc::new(StoredMembershipOf::<UTConfig>::new(membership_log_id, membership));

    RaftState::<UTConfig> {
        vote: Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), vote),
        log_ids: LogIdList::new(None, log_ids),
        membership_state: MembershipState::new(stored.clone(), stored),
        ..Default::default()
    }
}

#[test]
fn test_committed_leader_id_satisfied() {
    let st = state(Vote::new_committed(1, 2), vec![log_id(1, 2, 3)], Some(log_id(1, 2, 1)));

    let cond = Precondition::<UTConfig>::CommittedLeaderId {
        committed_leader_id: committed_leader_id(1, 2),
    };
    let got = cond.ensure_satisfied(&st);

    assert_eq!(Ok(()), got);
}

#[test]
fn test_committed_leader_id_mismatch() {
    let st = state(Vote::new_committed(1, 2), vec![log_id(1, 2, 3)], Some(log_id(1, 2, 1)));

    let cond = Precondition::<UTConfig>::CommittedLeaderId {
        committed_leader_id: committed_leader_id(2, 5),
    };
    let got = cond.ensure_satisfied(&st);

    let want = PreconditionFailed::CommittedLeaderIdMismatch {
        expected: committed_leader_id(2, 5),
        actual: Some(committed_leader_id(1, 2)),
    };
    assert_eq!(Err(want), got);
}

/// A node whose vote is not committed has no established leader, so no `LeaderId` is satisfied.
#[test]
fn test_committed_leader_id_mismatch_when_vote_is_not_committed() {
    let st = state(Vote::new(1, 2), vec![log_id(1, 2, 3)], Some(log_id(1, 2, 1)));

    let cond = Precondition::<UTConfig>::CommittedLeaderId {
        committed_leader_id: committed_leader_id(1, 2),
    };
    let got = cond.ensure_satisfied(&st);

    let want = PreconditionFailed::CommittedLeaderIdMismatch {
        expected: committed_leader_id(1, 2),
        actual: None,
    };
    assert_eq!(Err(want), got);
}

#[test]
fn test_last_log_id_satisfied() {
    let st = state(Vote::new_committed(1, 2), vec![log_id(1, 2, 3)], Some(log_id(1, 2, 1)));

    let cond = Precondition::<UTConfig>::LastLogId {
        last_log_id: Some(log_id(1, 2, 3)),
    };
    let got = cond.ensure_satisfied(&st);

    assert_eq!(Ok(()), got);
}

#[test]
fn test_last_log_id_satisfied_on_empty_log() {
    let st = state(Vote::new_committed(1, 2), vec![], None);

    let cond = Precondition::<UTConfig>::LastLogId { last_log_id: None };
    let got = cond.ensure_satisfied(&st);

    assert_eq!(Ok(()), got);
}

#[test]
fn test_last_log_id_mismatch() {
    let st = state(Vote::new_committed(1, 2), vec![log_id(1, 2, 3)], Some(log_id(1, 2, 1)));

    let cond = Precondition::<UTConfig>::LastLogId {
        last_log_id: Some(log_id(1, 2, 2)),
    };
    let got = cond.ensure_satisfied(&st);

    let want = PreconditionFailed::LastLogIdMismatch {
        expected: Some(log_id(1, 2, 2)),
        actual: Some(log_id(1, 2, 3)),
    };
    assert_eq!(Err(want), got);
}

#[test]
fn test_last_log_id_mismatch_on_empty_log() {
    let st = state(Vote::new_committed(1, 2), vec![], None);

    let cond = Precondition::<UTConfig>::LastLogId {
        last_log_id: Some(log_id(1, 2, 3)),
    };
    let got = cond.ensure_satisfied(&st);

    let want = PreconditionFailed::LastLogIdMismatch {
        expected: Some(log_id(1, 2, 3)),
        actual: None,
    };
    assert_eq!(Err(want), got);
}

#[test]
fn test_last_membership_log_id_satisfied() {
    let st = state(Vote::new_committed(1, 2), vec![log_id(1, 2, 3)], Some(log_id(1, 2, 1)));

    let cond = Precondition::<UTConfig>::LastMembershipLogId {
        last_membership_log_id: Some(log_id(1, 2, 1)),
    };
    let got = cond.ensure_satisfied(&st);

    assert_eq!(Ok(()), got);
}

#[test]
fn test_last_membership_log_id_mismatch() {
    let st = state(Vote::new_committed(1, 2), vec![log_id(1, 2, 3)], Some(log_id(1, 2, 3)));

    let cond = Precondition::<UTConfig>::LastMembershipLogId {
        last_membership_log_id: Some(log_id(1, 2, 1)),
    };
    let got = cond.ensure_satisfied(&st);

    let want = PreconditionFailed::LastMembershipLogIdMismatch {
        expected: Some(log_id(1, 2, 1)),
        actual: Some(log_id(1, 2, 3)),
    };
    assert_eq!(Err(want), got);
}

#[test]
fn test_display() {
    let cond = Precondition::<UTConfig>::CommittedLeaderId {
        committed_leader_id: committed_leader_id(1, 2),
    };
    assert_eq!("CommittedLeaderId(T1-N2)", cond.to_string());

    let cond = Precondition::<UTConfig>::LastLogId {
        last_log_id: Some(log_id(1, 2, 3)),
    };
    assert_eq!("LastLogId(T1-N2.3)", cond.to_string());

    let cond = Precondition::<UTConfig>::LastLogId { last_log_id: None };
    assert_eq!("LastLogId(None)", cond.to_string());

    let cond = Precondition::<UTConfig>::LastMembershipLogId {
        last_membership_log_id: Some(log_id(1, 2, 3)),
    };
    assert_eq!("LastMembershipLogId(T1-N2.3)", cond.to_string());

    let cond = Precondition::<UTConfig>::LastMembershipLogId {
        last_membership_log_id: None,
    };
    assert_eq!("LastMembershipLogId(None)", cond.to_string());
}
