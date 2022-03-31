use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::metrics::WaitError;
use openraft::Config;
use openraft::State;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Test wait() utils
///
/// What does this test do?
///
/// - brings 1 nodes online:
/// - wait for expected state.
/// - wait for invalid state and expect a timeout error.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn metrics_wait() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());

    let cluster = btreeset![0];
    router.new_raft_node(0).await;
    router.initialize_with(0, cluster.clone()).await?;
    router.wait_for_state(&cluster, State::Leader, timeout(), "init").await?;
    router.wait(&0, None).await?.current_leader(0, "become leader").await?;
    router.wait_for_log(&cluster, Some(1), None, "initial log").await?;

    tracing::info!("--- wait and timeout");

    let rst = router.wait(&0, timeout()).await?.log(Some(2), "timeout waiting for log 2").await;

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
