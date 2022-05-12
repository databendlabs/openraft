use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use memstore::IntoMemClientRequest;
use openraft::Config;
use openraft::LogIdOptionExt;
use openraft::Raft;
use openraft::RaftStorage;
use openraft::RaftTypeConfig;
use openraft::ServerState;
use tokio::time::sleep;

use crate::fixtures::init_default_ut_tracing;
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
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn learner_restart() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());

    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    let mut log_index = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1], None, None, "empty").await?;
    router.wait_for_state(&btreeset![0, 1], ServerState::Learner, None, "empty").await?;
    router.assert_pristine_cluster().await;

    tracing::info!("--- initializing single node cluster");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.initialize(btreeset! {0}).await?;
        log_index += 1;

        router.wait(&0, timeout()).state(ServerState::Leader, "n0 -> leader").await?;
    }

    router.add_learner(0, 1).await?;
    router.client_request(0, "foo", 1).await;
    log_index += 2;

    router.wait_for_log(&btreeset![0, 1], Some(log_index), None, "write one log").await?;

    let (node0, _sto0) = router.remove_node(0).await.unwrap();
    assert_node_state(0, &node0, 1, log_index, ServerState::Leader);
    node0.shutdown().await?;

    let (node1, sto1) = router.remove_node(1).await.unwrap();
    assert_node_state(0, &node1, 1, log_index, ServerState::Learner);
    node1.shutdown().await?;

    // restart node-1, assert the state as expected.
    let restarted = Raft::new(1, config.clone(), router.clone(), sto1);
    sleep(Duration::from_secs(2)).await;
    assert_node_state(1, &restarted, 1, log_index, ServerState::Learner);

    Ok(())
}

fn assert_node_state<C: RaftTypeConfig, S: RaftStorage<C>>(
    id: C::NodeId,
    node: &MemRaft<C, S>,
    expected_term: u64,
    expected_log: u64,
    state: ServerState,
) where
    C::D: Debug + IntoMemClientRequest<C::D>,
    C::R: Debug,
    S: Default + Clone,
{
    let m = node.metrics().borrow().clone();
    tracing::info!("node {} metrics: {:?}", id, m);

    assert_eq!(expected_term, m.current_term, "node {} term", id);
    assert_eq!(Some(expected_log), m.last_log_index, "node {} last_log_index", id);
    assert_eq!(Some(expected_log), m.last_applied.index(), "node {} last_log_index", id);
    assert_eq!(state, m.state, "node {} state", id);
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
