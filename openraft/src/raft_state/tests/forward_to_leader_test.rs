use std::sync::Arc;

use maplit::btreemap;
use maplit::btreeset;

use crate::error::ForwardToLeader;
use crate::utime::UTime;
use crate::CommittedLeaderId;
use crate::EffectiveMembership;
use crate::LogId;
use crate::Membership;
use crate::MembershipState;
use crate::RaftState;
use crate::TokioInstant;
use crate::Vote;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: CommittedLeaderId::new(term, 0),
        index,
    }
}

fn m12() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {1,2}], None)
}

#[test]
fn test_forward_to_leader_vote_not_committed() {
    let rs = RaftState {
        vote: UTime::new(TokioInstant::now(), Vote::new(1, 2)),
        membership_state: MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12())),
        ),
        ..Default::default()
    };

    assert_eq!(ForwardToLeader::empty(), rs.forward_to_leader());
}

#[test]
fn test_forward_to_leader_not_a_member() {
    let rs = RaftState {
        vote: UTime::new(TokioInstant::now(), Vote::new_committed(1, 3)),
        membership_state: MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12())),
        ),
        ..Default::default()
    };

    assert_eq!(ForwardToLeader::empty(), rs.forward_to_leader());
}

#[test]
fn test_forward_to_leader_has_leader() {
    let m123 = || Membership::<u64, u64>::new(vec![btreeset! {1,2}], btreemap! {1=>4,2=>5,3=>6});

    let rs = RaftState {
        vote: UTime::new(TokioInstant::now(), Vote::new_committed(1, 3)),
        membership_state: MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m123())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m123())),
        ),
        ..Default::default()
    };

    assert_eq!(ForwardToLeader::new(3, 6), rs.forward_to_leader());
}
