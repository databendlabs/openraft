use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::metrics::WaitError;

use crate::fixtures::RaftRouter;
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
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- bring up a single node cluster");
    let log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- wait for a log that is never written, expect timeout");

    let never_written = log_index + 1;
    let msg = format!("timeout waiting for log {}", never_written);
    let rst = router.wait(&0, timeout()).applied_index(Some(never_written), msg).await;

    match rst {
        Ok(_) => {
            panic!("expect timeout error");
        }
        Err(e) => {
            match e {
                WaitError::Timeout(_, _) => {
                    // ok
                }
                WaitError::ShuttingDown => {
                    panic!("unexpected error")
                }
            }
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
