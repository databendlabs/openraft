use std::time::Duration;

use anyhow::Result;
use openraft::Config;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Large heartbeat should not block replication.
/// I.e., replication should not be driven by heartbeat.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn large_heartbeat() -> Result<()> {
    // Setup test dependencies.
    let config = Config {
        heartbeat_interval: 10_000,
        election_timeout_min: 20_000,
        election_timeout_max: 30_000,
        max_payload_entries: 2,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    let mut log_index = router.new_cluster([0], [1]).await?;

    router.client_request_many(0, "foo", 10).await?;
    log_index += 10;

    router
        .wait_with_timeout(&1, Duration::from_millis(3_000))
        .applied_index(Some(log_index), "")
        .await?;

    Ok(())
}
