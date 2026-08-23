use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Cluster metrics_state_machine_consistency test.
///
/// What does this test do?
///
/// - brings 2 nodes online: one leader and one learner.
/// - write one log to the leader.
/// - asserts that when metrics.last_applied is up to date, the state machine should be up to date
///   too.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn metrics_state_machine_consistency() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- bring up one leader and one learner");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {1}).await?;

    tracing::info!(log_index, "--- write one log");
    router.client_request(0, "foo", 1).await?;

    // Wait for metrics to be up to date.
    // Once last_applied updated, the key should be visible in state machine.
    tracing::info!(log_index, "--- wait for log to sync");
    log_index += 1;
    for node_id in 0..2 {
        router.wait(&node_id, timeout()).applied_index(Some(log_index), "write one log").await?;
        let (_sto, sm) = router.get_storage_handle(&node_id)?;
        assert!(sm.get_state_machine().await.client_status.contains_key("foo"));
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
