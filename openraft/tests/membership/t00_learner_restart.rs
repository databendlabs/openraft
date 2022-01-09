use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::NodeId;
use openraft::Raft;
use openraft::State;
use tokio::time::sleep;

use crate::fixtures::MemRaft;
use crate::fixtures::RaftRouter;

/// Cluster learner_restart test.
///
/// What does this test do?
///
/// - brings 2 nodes online: one leader and one learner.
/// - write one log to the leader.
/// - asserts that the leader was able to successfully commit its initial payload and that the learner has successfully
///   replicated the payload.
/// - shutdown all and restart the learner node.
/// - asserts the learner stays in non-vtoer state.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn learner_restart() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    let mut n_logs = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1], n_logs, None, "empty").await?;
    router.wait_for_state(&btreeset![0, 1], State::Learner, None, "empty").await?;
    router.assert_pristine_cluster().await;

    tracing::info!("--- initializing single node cluster");

    router.initialize_with(0, btreeset![0]).await?;
    n_logs += 1;

    router.add_learner(0, 1).await?;
    router.client_request(0, "foo", 1).await;
    n_logs += 1;

    router.wait_for_log(&btreeset![0, 1], n_logs, None, "write one log").await?;

    let (node0, _sto0) = router.remove_node(0).await.unwrap();
    assert_node_state(0, &node0, 1, 2, State::Leader);
    node0.shutdown().await?;

    let (node1, sto1) = router.remove_node(1).await.unwrap();
    assert_node_state(0, &node1, 1, 2, State::Learner);
    node1.shutdown().await?;

    // restart node-1, assert the state as expected.
    let restarted = Raft::new(1, config.clone(), router.clone(), sto1);
    sleep(Duration::from_secs(2)).await;
    assert_node_state(1, &restarted, 1, 2, State::Learner);

    Ok(())
}

fn assert_node_state(id: NodeId, node: &MemRaft, expected_term: u64, expected_log: u64, state: State) {
    let m = node.metrics().borrow().clone();
    tracing::info!("node {} metrics: {:?}", id, m);

    assert_eq!(expected_term, m.current_term, "node {} term", id);
    assert_eq!(Some(expected_log), m.last_log_index, "node {} last_log_index", id);
    assert_eq!(expected_log, m.last_applied, "node {} last_log_index", id);
    assert_eq!(state, m.state, "node {} state", id);
}
