use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::stream::StreamExt;
use maplit::btreeset;
use openraft::Config;
use openraft::LogIdOptionExt;
use openraft::State;
use tokio::time::sleep;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Dynamic membership test.
///
/// What does this test do?
///
/// - bring a single-node cluster online.
/// - add a few new nodes and assert that they've joined the cluster properly.
/// - propose a new config change where the old master is not present, and assert that it steps down.
/// - temporarily isolate the new master, and assert that a new master takes over.
/// - restore the isolated node and assert that it becomes a follower.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn leader_election_after_changing_0_to_01234() -> Result<()> {
    let span = tracing::debug_span!("ut-dynamic_membership");
    let _ent = span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let mut n_logs = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0], None, None, "empty").await?;
    router.wait_for_state(&btreeset![0], State::Learner, None, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    n_logs += 1;

    router.wait_for_log(&btreeset![0], Some(n_logs), None, "init").await?;
    router.assert_stable_cluster(Some(1), Some(n_logs)).await;

    // Sync some new nodes.
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;
    router.new_raft_node(3).await;
    router.new_raft_node(4).await;

    tracing::info!("--- adding new nodes to cluster");
    let mut new_nodes = futures::stream::FuturesUnordered::new();
    new_nodes.push(router.add_learner(0, 1));
    new_nodes.push(router.add_learner(0, 2));
    new_nodes.push(router.add_learner(0, 3));
    new_nodes.push(router.add_learner(0, 4));
    while let Some(inner) = new_nodes.next().await {
        inner?;
    }

    n_logs += 4;
    router.wait_for_log(&btreeset![0], Some(n_logs), None, "cluster of 4 learners").await?;

    tracing::info!("--- changing cluster config");
    let node = router.get_raft_handle(&0)?;
    node.change_membership(btreeset![0, 1, 2, 3, 4], true, false).await?;
    n_logs += 2;

    router
        .wait_for_log(&btreeset![0, 1, 2, 3, 4], Some(n_logs), None, "cluster of 5 candidates")
        .await?;
    router.assert_stable_cluster(Some(1), Some(n_logs)).await; // Still in term 1, so leader is still node 0.

    // Isolate old leader and assert that a new leader takes over.
    tracing::info!("--- isolating master node 0");
    router.isolate_node(0).await;
    router
        .wait_for_metrics(
            &1,
            |x| x.current_leader.is_some() && x.current_leader.unwrap() != 0,
            Some(Duration::from_millis(1000)),
            "wait for new leader",
        )
        .await?;

    // need some time to stabilize.
    // TODO: it can not be sure that no new leader is elected after a leader detected on node-1
    // Wait for election and for everything to stabilize (this is way longer than needed).
    sleep(Duration::from_millis(1000)).await;

    let metrics = &router.latest_metrics()[1];
    let term = metrics.current_term;
    let applied = metrics.last_applied;
    let leader_id = metrics.current_leader;

    router.assert_stable_cluster(Some(term), applied.index()).await;
    let leader = router.leader().expect("expected new leader");
    assert!(leader != 0, "expected new leader to be different from the old leader");

    // Restore isolated node.
    router.restore_node(0).await;
    router
        .wait_for_metrics(
            &0,
            |x| x.current_leader == leader_id && x.last_applied == applied,
            Some(Duration::from_millis(1000)),
            "wait for restored node-0 to sync",
        )
        .await?;

    router.assert_stable_cluster(Some(term), applied.index()).await;

    let current_leader = router.leader().expect("expected to find current leader");
    assert_eq!(leader, current_leader, "expected cluster leadership to stay the same");

    Ok(())
}
