use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::LogId;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

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
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    // Write some data to the single node cluster.
    log_index += router.client_request_many(0, "0", 1000).await?;
    router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "client_request_many").await?;
    router
        .assert_storage_state(
            1,
            log_index,
            Some(0),
            LogId::new(CommittedLeaderId::new(1, 0), log_index),
            None,
        )
        .await?;

    // Read some data from the single node cluster.
    router.ensure_linearizable(0).await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
