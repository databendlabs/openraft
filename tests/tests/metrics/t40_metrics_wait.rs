use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::metrics::WaitError;

use crate::fixtures::RaftRouter;
use crate::fixtures::expect_timeout;
use crate::fixtures::ut_harness;

/// Test wait() utils
///
/// What does this test do?
///
/// - brings 1 nodes online:
/// - wait for expected state.
/// - wait for invalid state and expect a timeout error.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn metrics_wait() -> Result<()> {
    // Setup test dependencies.
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single node cluster");
    let log_index = router.new_cluster([0], []).await?;

    tracing::info!(log_index, "--- wait for a log that is never written, expect timeout");

    let never_written = log_index + 1;
    let msg = format!("timeout waiting for log {}", never_written);
    let duration = Duration::from_millis(20);
    let rst = router.wait_with_timeout(&0, duration).applied_index(Some(never_written), msg).await;
    expect_timeout(rst)?;

    tracing::info!("--- shutdown must fail a negative observation");
    {
        let node = router.get_raft_handle(&0)?;
        node.shutdown().await?;
        let result = router.wait(&0).applied_index(Some(never_written), "node stopped").await;
        let error = result.unwrap_err();
        assert_eq!(WaitError::ShuttingDown, error);
        let result = expect_timeout::<()>(Err(error));
        let error = result.unwrap_err();
        let wait_error = error.downcast_ref::<WaitError>();
        assert_eq!(Some(&WaitError::ShuttingDown), wait_error);
    }

    Ok(())
}
