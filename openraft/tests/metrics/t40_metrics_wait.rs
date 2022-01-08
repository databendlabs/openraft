use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::metrics::WaitError;
use openraft::Config;
use openraft::State;

use crate::fixtures::RaftRouter;

/// Test wait() utils
///
/// What does this test do?
///
/// - brings 1 nodes online:
/// - wait for expected state.
/// - wait for invalid state and expect a timeout error.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metrics_wait() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    let cluster = btreeset![0];
    router.new_raft_node(0).await;
    router.initialize_with(0, cluster.clone()).await?;
    router.wait_for_state(&cluster, State::Leader, None, "init").await?;
    router.wait(&0, None).await?.current_leader(0, "become leader").await?;
    router.wait_for_log(&cluster, 1, None, "initial log").await?;

    tracing::info!("--- wait and timeout");

    let rst = router.wait(&0, Some(Duration::from_millis(200))).await?.log(2, "timeout waiting for log 2").await;

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
