use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Client read tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - call the is_leader interface on the leader, and assert success.
/// - call the is_leader interface on the followers, and assert failure.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn client_reads() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    // This test is sensitive to network delay.
    router.network_send_delay(0);

    router.new_raft_node(0);
    router.new_raft_node(1);
    router.new_raft_node(2);

    let mut log_index = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1, 2], None, None, "empty node").await?;
    router.wait_for_state(&btreeset![0, 1, 2], ServerState::Learner, None, "empty node").await?;
    router.assert_pristine_cluster();

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    log_index += 1;

    router.wait_for_log(&btreeset![0, 1, 2], Some(log_index), None, "init leader").await?;
    router.assert_stable_cluster(Some(1), Some(1));

    // Get the ID of the leader, and assert that is_leader succeeds.
    let leader = router.leader().expect("leader not found");
    assert_eq!(leader, 0, "expected leader to be node 0, got {}", leader);
    router
        .is_leader(leader)
        .await
        .unwrap_or_else(|_| panic!("expected is_leader to succeed for cluster leader {}", leader));

    router.is_leader(1).await.expect_err("expected is_leader on follower node 1 to fail");
    router.is_leader(2).await.expect_err("expected is_leader on follower node 2 to fail");

    tracing::info!("--- isolate node 1 then is_leader should work");

    router.isolate_node(1);
    router.is_leader(leader).await?;

    tracing::info!("--- isolate node 2 then is_leader should fail");

    router.isolate_node(2);
    let rst = router.is_leader(leader).await;
    tracing::debug!(?rst, "is_leader with majority down");

    assert!(rst.is_err());

    Ok(())
}
