use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Raft;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Cluster learner_restart test.
///
/// What does this test do?
///
/// - brings 2 nodes online: one leader and one learner.
/// - write one log to the leader.
/// - asserts that the leader was able to successfully commit its initial payload and that the
///   learner has successfully replicated the payload.
/// - shutdown all and restart the learner node.
/// - asserts the learner stays in non-voter state.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn learner_restart() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {1}).await?;

    router.client_request(0, "foo", 1).await?;
    log_index += 1;

    router.wait_for_log(&btreeset![0, 1], Some(log_index), None, "write one log").await?;

    let (node0, _sto0, _sm0) = router.remove_node(0).unwrap();
    node0.shutdown().await?;

    let (node1, sto1, sm1) = router.remove_node(1).unwrap();
    node1.shutdown().await?;

    // restart node-1, assert the state as expected.
    let restarted = Raft::new(1, config.clone(), router.clone(), sto1, sm1).await?;
    restarted.wait(timeout()).applied_index(Some(log_index), "log after restart").await?;
    restarted.wait(timeout()).state(ServerState::Learner, "server state after restart").await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
