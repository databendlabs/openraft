use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::RaftRouter;

/// Large heartbeat should not block replication.
/// I.e., replication should not be driven by heartbeat.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn large_heartbeat() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

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
