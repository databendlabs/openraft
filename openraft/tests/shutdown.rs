use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::Config;
use openraft::State;

use crate::fixtures::init_default_ut_tracing;

#[macro_use]
mod fixtures;

/// Cluster shutdown test.
///
/// What does this test do?
///
/// - this test builds upon the `initialization` test.
/// - after the cluster has been initialize, it performs a shutdown routine on each node, asserting that the shutdown
///   routine succeeded.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn initialization() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let mut log_index = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1, 2], None, timeout(), "empty").await?;
    router.wait_for_state(&btreeset![0, 1, 2], State::Learner, timeout(), "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    log_index += 1;

    router.wait_for_log(&btreeset![0, 1, 2], Some(log_index), None, "init").await?;
    router.assert_stable_cluster(Some(1), Some(1)).await;

    tracing::info!("--- performing node shutdowns");
    {
        let (node0, _) = router.remove_node(0).await.ok_or_else(|| anyhow!("failed to find node 0 in router"))?;
        node0.shutdown().await?;

        let (node1, _) = router.remove_node(1).await.ok_or_else(|| anyhow!("failed to find node 1 in router"))?;
        node1.shutdown().await?;

        let (node2, _) = router.remove_node(2).await.ok_or_else(|| anyhow!("failed to find node 2 in router"))?;
        node2.shutdown().await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
