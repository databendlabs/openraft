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
/// - propose a new config change where the old master is not present, and assert that it steps
///   down.
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
    let mut log_index = router.new_cluster(btreeset! {0,1,2,3,4}, btreeset! {}).await?;

    // Isolate old leader and assert that a new leader takes over.
    tracing::info!(log_index, "--- isolating leader node 0");
    router.set_network_error(0, true);

    // Wait for leader lease to expire
    sleep(Duration::from_millis(700)).await;

    // Let node-1 become leader.
    let node_1 = router.get_raft_handle(&1)?;
    node_1.trigger().elect().await?;
    log_index += 1; // leader initial blank log

    router
        .wait_for_metrics(&1, |x| x.current_leader == Some(1), timeout(), "wait for new leader")
        .await?;

    for node_id in [1, 2, 3, 4] {
        router
            .wait(&node_id, timeout())
            .applied_index(Some(log_index), "replicate and apply log to every node")
            .await?;
    }

    let leader_id = 1;

    tracing::info!(log_index, "--- restore node 0, log_index:{}", log_index);
    router.set_network_error(0, false);
    router
        .wait(&0, timeout())
        .metrics(
            |x| x.current_leader == Some(leader_id) && x.last_applied.index() == Some(log_index),
            "wait for restored node-0 to sync",
        )
        .await?;

    let current_leader = router.leader().expect("expected to find current leader");
    assert_eq!(
        leader_id, current_leader,
        "expected cluster leadership to stay the same"
    );

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
