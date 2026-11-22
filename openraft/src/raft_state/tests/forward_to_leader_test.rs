use std::sync::Arc;
use std::time::Duration;

use maplit::btreemap;
use maplit::btreeset;

use crate::Membership;
use crate::MembershipState;
use crate::RaftState;
use crate::Vote;
use crate::engine::testing::UTConfig;
use crate::errors::ForwardToLeader;
use crate::impls::BasicNode;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::NodeIdOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;
use crate::vote::RaftLeaderIdExt;

fn log_id(term: u64, node_id: NodeIdOf<UTConfig<u64>>, index: u64) -> LogIdOf<UTConfig<u64>> {
    LogIdOf::<UTConfig<u64>>::new(LeaderIdOf::<UTConfig<u64>>::new_committed(term, node_id), index)
}

fn m12() -> Membership<u64, u64> {
    Membership::new_with_defaults(vec![btreeset! {1,2}], [])
}

#[test]
fn test_forward_to_leader_vote_not_committed() {
    let rs = RaftState::<UTConfig<u64>> {
        vote: Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(1, 2)),
        membership_state: MembershipState::new(
            Arc::new(StoredMembershipOf::<UTConfig<u64>>::new(Some(log_id(1, 0, 1)), m12())),
            Arc::new(StoredMembershipOf::<UTConfig<u64>>::new(Some(log_id(1, 0, 1)), m12())),
        ),
        ..Default::default()
    };

    assert_eq!(ForwardToLeader::empty(), rs.forward_to_leader());
}

#[test]
fn test_forward_to_leader_not_a_member() {
    let rs = RaftState::<UTConfig<u64>> {
        vote: Leased::new(
            UTConfig::<()>::now(),
            Duration::from_millis(500),
            Vote::new_committed(1, 3),
        ),
        membership_state: MembershipState::new(
            Arc::new(StoredMembershipOf::<UTConfig<u64>>::new(Some(log_id(1, 0, 1)), m12())),
            Arc::new(StoredMembershipOf::<UTConfig<u64>>::new(Some(log_id(1, 0, 1)), m12())),
        ),
        ..Default::default()
    };

    assert_eq!(ForwardToLeader::empty(), rs.forward_to_leader());
}

#[test]
fn test_forward_to_leader_has_leader() {
    let m123 = || Membership::<u64, u64>::new(vec![btreeset! {1,2}], btreemap! {1=>4,2=>5,3=>6}).unwrap();

    let rs = RaftState::<UTConfig<u64>> {
        vote: Leased::new(
            UTConfig::<()>::now(),
            Duration::from_millis(500),
            Vote::new_committed(1, 3),
        ),
        membership_state: MembershipState::new(
            Arc::new(StoredMembershipOf::<UTConfig<u64>>::new(Some(log_id(1, 0, 1)), m123())),
            Arc::new(StoredMembershipOf::<UTConfig<u64>>::new(Some(log_id(1, 0, 1)), m123())),
        ),
        ..Default::default()
    };

    assert_eq!(ForwardToLeader::new(3, 6), rs.forward_to_leader());
}

/// `UTConfig` takes node metadata that has no order, such as `BasicNode`.
#[test]
fn test_forward_to_leader_has_leader_with_basic_node() {
    let m123 = || {
        let nodes = btreemap! {1=>BasicNode::new("a1"), 2=>BasicNode::new("a2"), 3=>BasicNode::new("a3")};
        Membership::<u64, BasicNode>::new(vec![btreeset! {1,2}], nodes).unwrap()
    };

    let rs = RaftState::<UTConfig<BasicNode>> {
        vote: Leased::new(
            UTConfig::<()>::now(),
            Duration::from_millis(500),
            Vote::new_committed(1, 3),
        ),
        membership_state: MembershipState::new(
            Arc::new(StoredMembershipOf::<UTConfig<BasicNode>>::new(
                Some(log_id(1, 0, 1)),
                m123(),
            )),
            Arc::new(StoredMembershipOf::<UTConfig<BasicNode>>::new(
                Some(log_id(1, 0, 1)),
                m123(),
            )),
        ),
        ..Default::default()
    };

    assert_eq!(ForwardToLeader::new(3, BasicNode::new("a3")), rs.forward_to_leader());
}
