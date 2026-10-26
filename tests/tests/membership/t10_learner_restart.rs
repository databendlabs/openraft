use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Raft;
use openraft::ServerState;

use crate::fixtures::Node;
use crate::fixtures::RaftRouter;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

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
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
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

    for id in [0, 1] {
        router.wait(&id).applied_index(Some(log_index), "write one log").await?;
    }

    router.stop_node(0).await?;

    let Node {
        log_store: sto1,
        state_machine: sm1,
        ..
    } = router.stop_node(1).await?;

    // restart node-1, assert the state as expected.
    let restarted = Raft::new(1, config.clone(), router.clone(), sto1, sm1).await?;
    restarted.wait(timeout()).applied_index(Some(log_index), "log after restart").await?;
    restarted.wait(timeout()).state(ServerState::Learner, "server state after restart").await?;

    Ok(())
}
