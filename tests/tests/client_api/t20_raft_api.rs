use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::StepDownPolicy;
use openraft::Vote;
use openraft::async_runtime::watch::WatchReceiver;
use openraft::errors::ForwardReason;
use openraft::errors::ForwardToLeader;
use openraft::type_config::alias::LeaderIdOf;
use openraft::vote::RaftLeaderId;
use openraft_memstore::TypeConfig;

use crate::fixtures::Node;
use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Test Raft::is_leader() API
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn api_is_leader() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    router.new_cluster([0, 1, 2], []).await?;

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
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    router.new_cluster([0, 1, 2], []).await?;

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
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster with voters 0,1,2");
    router.new_cluster([0, 1, 2], []).await?;

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
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster with voters 0,1 and learners 2,3");
    let mut log_index = router.new_cluster([0, 1], [2, 3]).await?;

    let leader_id = router.leader().expect("leader not found");
    let leader = router.get_raft_handle(&leader_id)?;

    let learners: Vec<u64> = leader.learner_ids().collect();
    assert_eq!(vec![2, 3], learners);

    let voters: Vec<u64> = leader.voter_ids().collect();
    assert_eq!(vec![0, 1], voters);

    tracing::info!(
        log_index,
        "--- promote learner 2 and verify API IDs agree with its follower state"
    );
    {
        leader.change_membership(btreeset! {0,1,2}, true).await?;
        log_index += 1;

        router.wait(&leader_id).applied_index(Some(log_index), "promote learner 2").await?;
        router.wait(&2).state(openraft::ServerState::Follower, "promoted voter").await?;

        let learners: Vec<u64> = leader.learner_ids().collect();
        assert_eq!(vec![3], learners);
        let voters: Vec<u64> = leader.voter_ids().collect();
        assert_eq!(vec![0, 1, 2], voters);
    }

    Ok(())
}

/// Test all APIs work correctly after adding learner
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn api_after_add_learner() -> Result<()> {
    let config = Config {
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster with voters 0,1,2");
    router.new_cluster([0, 1, 2], []).await?;

    let leader_id = router.leader().expect("leader not found");
    let leader = router.get_raft_handle(&leader_id)?;

    // Initially no learners
    let learners: Vec<u64> = leader.learner_ids().collect();
    assert_eq!(learners.len(), 0, "should have 0 learners initially");

    // Add node 3 as learner
    tracing::info!("--- adding node 3 as learner");
    router.new_raft_node(3).await?;
    router.add_learner(leader_id, 3).await?;
    router.wait(&3).state(openraft::ServerState::Learner, "become learner").await?;

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
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    router.new_cluster([0, 1, 2], []).await?;

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

/// A leader that the membership keeps only as a learner still leads: `as_leader()` returns it, and
/// the followers forward to it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn api_as_leader_on_learner_leader() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster with voters 0,1,2");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- change voters to 1,2 and keep leader node-0 as a learner"
    );
    {
        n0.change_membership([1, 2], true).await?;
        log_index += 1;

        let metrics = router.wait(&0).applied_index(Some(log_index), "node-0 becomes a learner").await?;
        assert_eq!(ServerState::Leader, metrics.state, "node-0 keeps leading");
        assert_eq!(vec![0], n0.learner_ids().collect::<Vec<_>>());
        assert_eq!(vec![1, 2], n0.voter_ids().collect::<Vec<_>>());
    }

    tracing::info!(
        log_index,
        "--- as_leader() returns learner node-0 in term 1, and the followers forward to it"
    );
    {
        let leader = n0.as_leader().expect("a leader kept as a learner still leads");
        assert_eq!(&LeaderIdOf::<TypeConfig>::new(1, 0), leader.leader_id());

        let want = ForwardToLeader {
            leader_id: Some(0),
            leader_node: Some(()),
            reason: ForwardReason::NotLeader,
        };
        for id in [1, 2] {
            let forward = router.get_raft_handle(&id)?.as_leader().expect_err("a follower does not lead");
            assert_eq!(want, forward, "node-{id} forwards to node-0");
        }
    }

    Ok(())
}

/// A removed leader keeps its committed vote when it steps down to a learner, but no longer leads:
/// `as_leader()` then names no leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn api_as_leader_after_removed_leader_steps_down() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        removed_leader_step_down: StepDownPolicy::Never,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster with voters 0,1,2");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- remove leader node-0: it keeps leading until it is told to step down"
    );
    {
        n0.change_membership([1, 2], false).await?;
        log_index += 1;

        let metrics = router.wait(&0).applied_index(Some(log_index), "node-0 applies its removal").await?;
        assert_eq!(ServerState::Leader, metrics.state, "node-0 keeps leading");

        let leader = n0.as_leader().expect("a removed leader that has not stepped down still leads");
        assert_eq!(&LeaderIdOf::<TypeConfig>::new(1, 0), leader.leader_id());
    }

    tracing::info!(
        log_index,
        "--- step node-0 down: it keeps its committed vote of term 1, and as_leader() names no leader"
    );
    {
        n0.trigger().refresh_server_state(None, None).await?;
        let metrics = router.wait(&0).state(ServerState::Learner, "node-0 steps down").await?;
        assert_eq!(Vote::new_committed(1, 0), metrics.vote);

        let forward = n0.as_leader().expect_err("a removed leader that stepped down does not lead");
        assert_eq!(ForwardToLeader::empty(), forward);
    }

    Ok(())
}

/// A stopped leader, and a leader restarted without restoring its leadership, keep their committed
/// vote but do not lead: `as_leader()` names no leader until the node is elected again.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn api_as_leader_after_shutdown_and_passive_restart() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        enable_leader_restore: Some(false),
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let log_index = router.new_cluster([0], []).await?;

    tracing::info!(log_index, "--- node-0 leads in term 1: as_leader() returns it");
    {
        let n0 = router.get_raft_handle(&0)?;
        let leader = n0.as_leader().expect("the leader of a running cluster leads");
        assert_eq!(&LeaderIdOf::<TypeConfig>::new(1, 0), leader.leader_id());
    }

    tracing::info!(
        log_index,
        "--- stop node-0: it keeps its committed vote of term 1, and as_leader() names no leader"
    );
    let node = {
        let node = router.stop_node(0).await?;
        let metrics = node.raft.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Shutdown, metrics.state);
        assert_eq!(Vote::new_committed(1, 0), metrics.vote);

        let forward = node.raft.as_leader().expect_err("a stopped node does not lead");
        assert_eq!(ForwardToLeader::empty(), forward);
        node
    };

    tracing::info!(
        log_index,
        "--- restart node-0 as a follower that keeps its committed vote: as_leader() names no leader"
    );
    {
        let Node {
            log_store,
            state_machine,
            ..
        } = node;
        router.new_raft_node_with_sto(0, log_store, state_machine).await?;

        let want = Vote::new_committed(1, 0);
        router
            .wait(&0)
            .metrics(
                |m| m.state == ServerState::Follower && m.vote == want,
                "node-0 restarts as a follower with its committed vote",
            )
            .await?;

        let n0 = router.get_raft_handle(&0)?;
        let forward = n0.as_leader().expect_err("a restarted follower does not lead");
        assert_eq!(ForwardToLeader::empty(), forward);
    }

    tracing::info!(
        log_index,
        "--- elect node-0 again: as_leader() returns it with its leader ID of term 2"
    );
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().elect(false).await?;

        let want = Vote::new_committed(2, 0);
        router
            .wait(&0)
            .metrics(
                |m| m.state == ServerState::Leader && m.vote == want,
                "node-0 leads in term 2",
            )
            .await?;

        let leader = n0.as_leader().expect("an elected node leads");
        assert_eq!(&LeaderIdOf::<TypeConfig>::new(2, 0), leader.leader_id());
    }

    Ok(())
}

/// `Raft::runtime_config()` reads back the switches it sets, starting from `Config`.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn api_runtime_config_reads_back_switches() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: true,
        enable_elect: false,
        enable_pre_vote: Some(true),
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing a single node cluster");
    router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!("--- the switches start as Config sets them");
    {
        let rc = n0.runtime_config();
        assert!(!rc.is_tick_enabled());
        assert!(rc.is_heartbeat_enabled());
        assert!(!rc.is_elect_enabled());
        assert!(rc.is_pre_vote_enabled());
    }

    tracing::info!("--- every setter is read back");
    {
        let rc = n0.runtime_config();
        rc.tick(true);
        rc.heartbeat(false);
        rc.elect(true);
        rc.pre_vote(false);

        assert!(rc.is_tick_enabled());
        assert!(!rc.is_heartbeat_enabled());
        assert!(rc.is_elect_enabled());
        assert!(!rc.is_pre_vote_enabled());
    }

    Ok(())
}
