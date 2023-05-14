use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

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

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    // Get the ID of the leader, and assert that is_leader succeeds.
    let leader = router.leader().expect("leader not found");
    assert_eq!(leader, 0, "expected leader to be node 0, got {}", leader);
    router
        .is_leader(leader)
        .await
        .unwrap_or_else(|_| panic!("expected is_leader to succeed for cluster leader {}", leader));

    router.is_leader(1).await.expect_err("expected is_leader on follower node 1 to fail");
    router.is_leader(2).await.expect_err("expected is_leader on follower node 2 to fail");

    tracing::info!(log_index, "--- isolate node 1 then is_leader should work");

    router.set_node_network_failure(1, true);
    router.is_leader(leader).await?;

    tracing::info!(log_index, "--- isolate node 2 then is_leader should fail");

    router.set_node_network_failure(2, true);
    let rst = router.is_leader(leader).await;
    tracing::debug!(?rst, "is_leader with majority down");

    assert!(rst.is_err());

    Ok(())
}
