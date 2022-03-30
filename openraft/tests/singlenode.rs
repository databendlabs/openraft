use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::State;

use crate::fixtures::init_default_ut_tracing;

#[macro_use]
mod fixtures;

/// Single-node cluster initialization test.
///
/// What does this test do?
///
/// - brings 1 node online with only knowledge of itself.
/// - asserts that it remains in learner state with no activity (it should be completely passive).
/// - initializes the cluster with membership config including just the one node.
/// - asserts that the cluster was able to come online, and that the one node became leader.
/// - asserts that the leader was able to successfully commit its initial payload.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn single_node() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let mut n_logs = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0], None, timeout(), "empty").await?;
    router.wait_for_state(&btreeset![0], State::Learner, timeout(), "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    n_logs += 1;

    router.wait_for_log(&btreeset![0], Some(n_logs), timeout(), "init").await?;
    router.assert_stable_cluster(Some(1), Some(1)).await;

    // Write some data to the single node cluster.
    router.client_request_many(0, "0", 1000).await;
    n_logs += 1000;
    router.wait_for_log(&btreeset![0], Some(n_logs), timeout(), "client_request_many").await?;
    router.assert_stable_cluster(Some(1), Some(n_logs)).await;
    router
        .assert_storage_state(1, n_logs, Some(0), LogId::new(LeaderId::new(1, 0), n_logs), None)
        .await?;

    // Read some data from the single node cluster.
    router.is_leader(0).await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
