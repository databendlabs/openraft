use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LogIdOptionExt;
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
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_nodes_from_single(btreeset! {0,1,2,3,4}, btreeset! {}).await?;

    router.assert_stable_cluster(Some(1), Some(log_index)); // Still in term 1, so leader is still node 0.

    // Isolate old leader and assert that a new leader takes over.
    tracing::info!("--- isolating master node 0");
    router.isolate_node(0);

    // Let node-1 become leader.
    let node_1 = router.get_raft_handle(&1)?;
    node_1.enable_elect(true);

    router
        .wait_for_metrics(&1, |x| x.current_leader == Some(1), timeout(), "wait for new leader")
        .await?;

    // need some time to stabilize.
    // TODO: it can not be sure that no new leader is elected after a leader detected on node-1
    // Wait for election and for everything to stabilize (this is way longer than needed).
    sleep(Duration::from_millis(1000)).await;

    let metrics = &router.latest_metrics()[1];
    let term = metrics.current_term;
    let applied = metrics.last_applied;
    let leader_id = metrics.current_leader;

    router.assert_stable_cluster(Some(term), applied.index());
    let leader = router.leader().expect("expected new leader");

    tracing::info!("--- restore node 0");
    router.restore_node(0);
    router
        .wait_for_metrics(
            &0,
            |x| x.current_leader == leader_id && x.last_applied == applied,
            timeout(),
            "wait for restored node-0 to sync",
        )
        .await?;

    router.assert_stable_cluster(Some(term), applied.index());

    let current_leader = router.leader().expect("expected to find current leader");
    assert_eq!(leader, current_leader, "expected cluster leadership to stay the same");

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
