use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
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

    let cluster = btreeset![0];
    router.new_raft_node(0).await;
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.initialize(cluster.clone()).await?;

        router.wait(&0, timeout()).state(ServerState::Leader, "n0 -> leader").await?;
    }

    router.wait(&0, None).current_leader(0, "become leader").await?;
    router.wait(&0, None).applied_index(Some(1), "initial log").await?;

    tracing::info!("--- wait and timeout");

    let rst = router.wait(&0, timeout()).applied_index(Some(2), "timeout waiting for log 2").await;

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
