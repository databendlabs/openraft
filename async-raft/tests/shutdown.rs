use std::sync::Arc;

use anyhow::anyhow;
use anyhow::Result;
use async_raft::Config;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::btreeset;

#[macro_use]
mod fixtures;

/// Cluster shutdown test.
///
/// What does this test do?
///
/// - this test builds upon the `initialization` test.
/// - after the cluster has been initialize, it performs a shutdown routine on each node, asserting that the shutdown
///   routine succeeded.
///
/// RUST_LOG=async_raft,memstore,shutdown=trace cargo test -p async-raft --test shutdown
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn initialization() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let mut want = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1, 2], want, None, "empty").await?;
    router.wait_for_state(&btreeset![0, 1, 2], State::Learner, None, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    want += 1;

    router.wait_for_log(&btreeset![0, 1, 2], want, None, "init").await?;
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
