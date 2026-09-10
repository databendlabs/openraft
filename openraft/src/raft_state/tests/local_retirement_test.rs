use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use crate::Membership;
use crate::MembershipState;
use crate::RaftState;
use crate::ServerState;
use crate::Vote;
use crate::engine::testing::UTConfig;
use crate::errors::ForwardToLeader;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;
use crate::vote::RaftLeaderId;

fn state_with_membership(membership: Membership<u64, ()>) -> RaftState<UTConfig> {
    let stored = Arc::new(StoredMembershipOf::<UTConfig>::new(None, membership));

    RaftState {
        vote: Leased::new(
            UTConfig::<()>::now(),
            Duration::from_millis(500),
            Vote::new_committed(3, 1),
        ),
        membership_state: MembershipState::new(stored.clone(), stored),
        ..Default::default()
    }
}

#[test]
fn test_local_retirement_matches_only_one_leader_authority() {
    let mut state = state_with_membership(Membership::new_with_defaults(vec![btreeset! {1, 2}], []));
    state.locally_retired_for = Some(LeaderIdOf::<UTConfig>::new(3, 1));

    assert!(state.is_locally_retired(&1));
    assert!(!state.is_leading(&1));
    assert!(!state.is_leader(&1));
    assert_eq!(ServerState::Follower, state.calc_server_state(&1));
    assert_eq!(ForwardToLeader::empty(), state.forward_to_leader(&1));

    state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(3, 1));

    assert!(!state.is_locally_retired(&1));
    assert_eq!(ServerState::Candidate, state.calc_server_state(&1));

    state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(3, 2),
    );
    state.locally_retired_for = Some(LeaderIdOf::<UTConfig>::new(3, 2));

    assert!(!state.is_locally_retired(&1));
    assert_eq!(ServerState::Follower, state.calc_server_state(&1));
    assert_eq!(ForwardToLeader::new(2, ()), state.forward_to_leader(&1));

    state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(4, 1),
    );
    state.locally_retired_for = Some(LeaderIdOf::<UTConfig>::new(3, 1));

    assert!(!state.is_locally_retired(&1));
    assert!(state.is_leading(&1));
    assert!(state.is_leader(&1));
    assert_eq!(ServerState::Leader, state.calc_server_state(&1));
}

#[test]
fn test_local_retirement_of_learner_leader_becomes_learner() {
    let mut state = state_with_membership(Membership::new_with_defaults(vec![btreeset! {2, 3}], btreeset! {1}));
    state.locally_retired_for = Some(LeaderIdOf::<UTConfig>::new(3, 1));

    assert!(state.is_locally_retired(&1));
    assert_eq!(ServerState::Learner, state.calc_server_state(&1));
}
