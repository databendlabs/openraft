mod fixtures;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_raft::Config;
use tokio::time::sleep;

use fixtures::RaftRouter;

/// Current leader tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - call the current_leader interface on the all nodes, and assert success.
///
/// RUST_LOG=async_raft,memstore,client_reads=trace cargo test -p async-raft --test current_leader
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_leader() -> Result<()> {
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

    // Get the ID of the leader, and assert that current_leader succeeds.
    let leader = router.leader().await.expect("leader not found");
    assert_eq!(leader, 0, "expected leader to be node 0, got {}", leader);

    for i in 0..3 {
        let leader = router.current_leader(i).await;
        assert_eq!(leader, Some(0), "expected leader to be node 0, got {:?}", leader);
    }

    Ok(())
}
