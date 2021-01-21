mod fixtures;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_raft::Config;
use tokio::time::sleep;

use fixtures::RaftRouter;

/// Client read tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - call the client_read interface on the leader, and assert success.
/// - call the client_read interface on the followers, and assert failure.
///
/// RUST_LOG=async_raft,memstore,client_reads=trace cargo test -p async-raft --test client_reads
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_reads() -> Result<()> {
    fixtures::init_tracing();

    // Setup test dependencies.
    let config = Arc::new(Config::build("test".into()).validate().expect("failed to build Raft config"));
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    // Assert all nodes are in non-voter state & have no entries.
    sleep(Duration::from_secs(10)).await;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    sleep(Duration::from_secs(10)).await;
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

    Ok(())
}
