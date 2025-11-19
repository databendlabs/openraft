use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::async_runtime::watch::WatchReceiver;
use openraft::type_config::alias::LeaderIdOf;
use openraft::vote::RaftLeaderId;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Test Raft::is_leader() API
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn api_is_leader() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let leader_id = router.leader().expect("leader not found");
    let leader = router.get_raft_handle(&leader_id)?;

    // Leader should return true
    assert!(leader.is_leader(), "leader node should return true for is_leader()");

    // Followers should return false
    for follower_id in [0, 1, 2].iter().filter(|&&id| id != leader_id) {
        let follower = router.get_raft_handle(follower_id)?;
        assert!(
            !follower.is_leader(),
            "follower node {} should return false for is_leader()",
            follower_id
        );
    }

    Ok(())
}

/// Test Raft::node_id() API
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn api_node_id() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    // Each node should return its own ID
    for id in [0, 1, 2] {
        let node = router.get_raft_handle(&id)?;
        assert_eq!(node.node_id(), &id, "node {} should return its own ID", id);
    }

    Ok(())
}

/// Test Raft::voter_ids() API
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn api_voter_ids() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster with voters 0,1,2");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let leader_id = router.leader().expect("leader not found");
    let leader = router.get_raft_handle(&leader_id)?;

    let voters: Vec<u64> = leader.voter_ids().collect();
    assert_eq!(voters.len(), 3, "should have 3 voters");
    assert!(voters.contains(&0), "should contain voter 0");
    assert!(voters.contains(&1), "should contain voter 1");
    assert!(voters.contains(&2), "should contain voter 2");

    Ok(())
}

/// Test Raft::learner_ids() API
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn api_learner_ids() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster with voters 0,1 and learners 2,3");
    router.new_cluster(btreeset! {0,1}, btreeset! {2,3}).await?;

    let leader_id = router.leader().expect("leader not found");
    let leader = router.get_raft_handle(&leader_id)?;

    let learners: Vec<u64> = leader.learner_ids().collect();
    assert_eq!(learners.len(), 2, "should have 2 learners");
    assert!(learners.contains(&2), "should contain learner 2");
    assert!(learners.contains(&3), "should contain learner 3");

    // Verify voters and learners are separate
    let voters: Vec<u64> = leader.voter_ids().collect();
    assert_eq!(voters.len(), 2, "should have 2 voters");
    assert!(voters.contains(&0), "should contain voter 0");
    assert!(voters.contains(&1), "should contain voter 1");

    // Learners should not be in voters
    for learner in &learners {
        assert!(!voters.contains(learner), "learner {} should not be in voters", learner);
    }

    Ok(())
}

/// Test all APIs work correctly after adding learner
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn api_after_add_learner() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster with voters 0,1,2");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let leader_id = router.leader().expect("leader not found");
    let leader = router.get_raft_handle(&leader_id)?;

    // Initially no learners
    let learners: Vec<u64> = leader.learner_ids().collect();
    assert_eq!(learners.len(), 0, "should have 0 learners initially");

    // Add node 3 as learner
    tracing::info!("--- adding node 3 as learner");
    router.new_raft_node(3).await;
    router.add_learner(leader_id, 3).await?;
    router.wait(&3, None).state(openraft::ServerState::Learner, "become learner").await?;

    // Check learners after adding node 3
    let learners: Vec<u64> = leader.learner_ids().collect();
    assert_eq!(learners.len(), 1, "should have 1 learner after adding node 3");
    assert!(learners.contains(&3), "should contain learner 3");

    // Voters should remain unchanged
    let voters: Vec<u64> = leader.voter_ids().collect();
    assert_eq!(voters.len(), 3, "should still have 3 voters");
    assert!(!voters.contains(&3), "node 3 should not be a voter");

    Ok(())
}

/// Test Raft::as_leader() API
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn api_as_leader() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let leader_id = router.leader().expect("leader not found");
    let leader = router.get_raft_handle(&leader_id)?;

    // Leader should return Some(Leader)
    let metrics_rx = leader.metrics();
    let metrics = metrics_rx.borrow_watched();
    let expected_id = LeaderIdOf::<TypeConfig>::new(metrics.current_term, leader_id);

    let metrics = metrics_rx.borrow_watched().clone();

    let leader_info = leader.as_leader().expect("leader should return Leader info");
    assert_eq!(leader_info.leader_id(), &expected_id);
    assert!(leader_info.last_quorum_acked() >= metrics.last_quorum_acked.map(|s| s.into_inner()));

    // Followers should return Err(ForwardToLeader) with leader info
    for follower_id in [0, 1, 2].iter().filter(|&&id| id != leader_id) {
        let follower = router.get_raft_handle(follower_id)?;
        let forward = follower.as_leader().expect_err("follower node should return ForwardToLeader error");
        assert_eq!(
            forward.leader_id,
            Some(leader_id),
            "follower node {} should return ForwardToLeader with leader_id",
            follower_id
        );
        assert!(
            forward.leader_node.is_some(),
            "follower node {} should return ForwardToLeader with leader_node",
            follower_id
        );
    }

    Ok(())
}
