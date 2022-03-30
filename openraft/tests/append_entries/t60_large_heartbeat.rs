use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Large heartbeat should not block replication.
/// I.e., replication should not be driven by heartbeat.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn large_heartbeat() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            heartbeat_interval: 10_000,
            election_timeout_min: 20_000,
            election_timeout_max: 30_000,
            max_payload_entries: 2,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {1}).await?;

    router.client_request_many(0, "foo", 100).await;
    log_index += 100;

    router.wait(&1, Some(Duration::from_millis(1000))).await?.log(Some(log_index), "").await?;

    Ok(())
}
