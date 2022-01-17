use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::State;

use crate::fixtures::RaftRouter;

/// Client read tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - call the client_read interface on the leader, and assert success.
/// - call the client_read interface on the followers, and assert failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_reads() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let mut n_logs = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1, 2], None, None, "empty node").await?;
    router.wait_for_state(&btreeset![0, 1, 2], State::Learner, None, "empty node").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    n_logs += 1;

    router.wait_for_log(&btreeset![0, 1, 2], Some(n_logs), None, "init leader").await?;
    router.assert_stable_cluster(Some(1), Some(1)).await;

    // Get the ID of the leader, and assert that client_read succeeds.
    let leader = router.leader().await.expect("leader not found");
    assert_eq!(leader, 0, "expected leader to be node 0, got {}", leader);
    router
        .client_read(leader)
        .await
        .unwrap_or_else(|_| panic!("expected client_read to succeed for cluster leader {}", leader));

    router.client_read(1).await.expect_err("expected client_read on follower node 1 to fail");
    router.client_read(2).await.expect_err("expected client_read on follower node 2 to fail");

    tracing::info!("--- isolate node 1 then client read should work");

    router.isolate_node(1).await;
    router.client_read(leader).await?;

    tracing::info!("--- isolate node 2 then client read should fail");

    router.isolate_node(2).await;
    let rst = router.client_read(leader).await;
    tracing::debug!(?rst, "client_read with majority down");

    assert!(rst.is_err());

    Ok(())
}
