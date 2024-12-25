use std::sync::Arc;
use std::time::Duration;

use maplit::btreemap;
use maplit::btreeset;

use crate::engine::testing::UTConfig;
use crate::error::ForwardToLeader;
use crate::testing::log_id;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;
use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::RaftState;
use crate::Vote;

fn m12() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {1,2}], [])
}

#[test]
fn test_forward_to_leader_vote_not_committed() {
    let rs = RaftState::<UTConfig> {
        vote: Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(1, 2)),
        membership_state: MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 0, 1)), m12())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 0, 1)), m12())),
        ),
        ..Default::default()
    };

    assert_eq!(ForwardToLeader::empty(), rs.forward_to_leader());
}

#[test]
fn test_forward_to_leader_not_a_member() {
    let rs = RaftState::<UTConfig> {
        vote: Leased::new(
            UTConfig::<()>::now(),
            Duration::from_millis(500),
            Vote::new_committed(1, 3),
        ),
        membership_state: MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 0, 1)), m12())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 0, 1)), m12())),
        ),
        ..Default::default()
    };

    assert_eq!(ForwardToLeader::empty(), rs.forward_to_leader());
}

#[test]
fn test_forward_to_leader_has_leader() {
    let m123 = || Membership::<UTConfig<u64>>::new(vec![btreeset! {1,2}], btreemap! {1=>4,2=>5,3=>6}).unwrap();

    let rs = RaftState::<UTConfig<u64>> {
        vote: Leased::new(
            UTConfig::<()>::now(),
            Duration::from_millis(500),
            Vote::new_committed(1, 3),
        ),
        membership_state: MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 0, 1)), m123())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 0, 1)), m123())),
        ),
        ..Default::default()
    };

    assert_eq!(ForwardToLeader::new(3, 6), rs.forward_to_leader());
}
