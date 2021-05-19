use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::hashset;
use tokio::time::sleep;

use async_raft::{Config, NodeId, Raft, State};
use fixtures::RaftRouter;

use crate::fixtures::MemRaft;

mod fixtures;

/// Cluster non_voter_restart test.
///
/// What does this test do?
///
/// - brings 2 nodes online: one leader and one non-voter.
/// - write one log to the leader.
/// - asserts that the leader was able to successfully commit its initial payload and that the
///   non-voter has successfully replicated the payload.
/// - shutdown all and retstart the non-voter node.
/// - asserts the non-voter stays in non-vtoer state.
///
/// RUST_LOG=async_raft,memstore,non_voter_restart=trace cargo test -p async-raft --test non_voter_restart
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn non_voter_restart() -> Result<()> {
    fixtures::init_tracing();

    // Setup test dependencies.
    let config = Arc::new(Config::build("test".into()).validate().expect("failed to build Raft config"));
    let router = Arc::new(RaftRouter::new(config.clone()));

    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    // Assert all nodes are in non-voter state & have no entries.
    sleep(Duration::from_secs(2)).await;
    router.assert_pristine_cluster().await;

    tracing::info!("--- initializing single node cluster");

    router.initialize_with(0, hashset![0]).await?;
    router.add_non_voter(0, 1).await?;
    router.client_request(0, "foo", 1).await;
    sleep(Duration::from_secs(2)).await;

    let (node0, _sto0) = router.remove_node(0).await.unwrap();
    assert_node_state(0, &node0, 1, 2, State::Leader);
    node0.shutdown().await?;

    let (node1, sto1) = router.remove_node(1).await.unwrap();
    assert_node_state(0, &node1, 1, 2, State::NonVoter);
    node1.shutdown().await?;

    // restart node-1, assert the state as expected.
    let restarted = Raft::new(1, config.clone(), router.clone(), sto1);
    sleep(Duration::from_secs(2)).await;
    assert_node_state(1, &restarted, 1, 2, State::NonVoter);

    Ok(())
}

fn assert_node_state(id: NodeId, node: &MemRaft, expected_term: u64, expected_log: u64, state: State) {
    let m = node.metrics().borrow().clone();
    tracing::info!("node {} metrics: {:?}", id, m);

    assert_eq!(expected_term, m.current_term, "node {} term", id);
    assert_eq!(expected_log, m.last_log_index, "node {} last_log_index", id);
    assert_eq!(expected_log, m.last_applied, "node {} last_log_index", id);
    assert_eq!(state, m.state, "node {} state", id);
}
