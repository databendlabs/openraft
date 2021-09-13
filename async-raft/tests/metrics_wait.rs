use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_raft::metrics::WaitError;
use async_raft::Config;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::btreeset;

#[macro_use]
mod fixtures;

/// Test wait() utils
///
/// What does this test do?
///
/// - brings 1 nodes online:
/// - wait for expected state.
/// - wait for invalid state and expect a timeout error.
///
/// RUST_LOG=async_raft,memstore,metrics_wait=trace cargo test -p async-raft --test metrics_wait
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metrics_wait() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate().expect("failed to build Raft config"));
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
                WaitError::RaftError(_) => {
                    panic!("unexpected error")
                }
            }
        }
    }

    Ok(())
}
