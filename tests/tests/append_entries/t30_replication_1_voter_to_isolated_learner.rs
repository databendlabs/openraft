use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Test replication to learner that is not in membership should not block.
///
/// What does this test do?
///
/// - bring on a cluster of 1 voter and 1 learner.
/// - isolate replication to node 1.
/// - client write should not be blocked.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn replication_1_voter_to_isolated_learner() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {1}).await?;

    tracing::info!(log_index, "--- stop replication to node 1");
    {
        router.set_network_error(1, true);

        router.client_request_many(0, "0", (10 - log_index) as usize).await?;
        log_index = 10;

        router
            .wait_for_log(
                &btreeset![0],
                Some(log_index),
                timeout(),
                "send log to trigger snapshot",
            )
            .await?;
    }

    tracing::info!(log_index, "--- restore replication to node 1");
    {
        router.set_network_error(1, false);

        router.client_request_many(0, "0", (10 - log_index) as usize).await?;
        log_index = 10;

        router
            .wait_for_log(
                &btreeset![0],
                Some(log_index),
                timeout(),
                "send log to trigger snapshot",
            )
            .await?;
    }
    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
