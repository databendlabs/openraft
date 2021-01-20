mod fixtures;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_raft::Config;
use tokio::time::sleep;

use fixtures::RaftRouter;

/// Cluster shutdown test.
///
/// What does this test do?
///
/// - this test builds upon the `initialization` test.
/// - after the cluster has been initialize, it performs a shutdown routine
///   on each node, asserting that the shutdown routine succeeded.
///
/// RUST_LOG=async_raft,memstore,shutdown=trace cargo test -p async-raft --test shutdown
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn initialization() -> Result<()> {
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

    tracing::info!("--- performing node shutdowns");
    let (node0, _) = router.remove_node(0).await.ok_or_else(|| anyhow!("failed to find node 0 in router"))?;
    node0.shutdown().await?;
    let (node1, _) = router.remove_node(1).await.ok_or_else(|| anyhow!("failed to find node 1 in router"))?;
    node1.shutdown().await?;
    let (node2, _) = router.remove_node(2).await.ok_or_else(|| anyhow!("failed to find node 2 in router"))?;
    node2.shutdown().await?;

    Ok(())
}
