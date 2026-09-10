use super::local_leader_id;
use crate::ServerState;
use crate::Vote;
use crate::engine::testing::UTConfig;
use crate::metrics::RaftServerMetrics;

#[test]
fn test_local_leader_id_requires_leader_state_and_local_committed_vote() {
    let mut metrics = RaftServerMetrics::<UTConfig>::new_initial(1);
    metrics.vote = Vote::new_committed(3, 1);
    metrics.state = ServerState::Leader;
    let leader_id = *metrics.vote.leader_id();

    assert_eq!(Some(leader_id), local_leader_id(&metrics));

    metrics.state = ServerState::Follower;
    assert_eq!(None, local_leader_id(&metrics));

    metrics.state = ServerState::Leader;
    metrics.vote = Vote::new(4, 1);
    assert_eq!(None, local_leader_id(&metrics));

    metrics.vote = Vote::new_committed(4, 2);
    assert_eq!(None, local_leader_id(&metrics));
}
