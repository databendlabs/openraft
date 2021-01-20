mod fixtures;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_raft::Config;
use futures::stream::StreamExt;
use maplit::hashset;
use tokio::time::sleep;

use fixtures::RaftRouter;

/// Dynamic membership test.
///
/// What does this test do?
///
/// - bring a single-node cluster online.
/// - add a few new nodes and assert that they've joined the cluster properly.
/// - propose a new config change where the old master is not present, and assert that it steps down.
/// - temporarily isolate the new master, and assert that a new master takes over.
/// - restore the isolated node and assert that it becomes a follower.
///
/// RUST_LOG=async_raft,memstore,dynamic_membership=trace cargo test -p async-raft --test dynamic_membership
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn dynamic_membership() -> Result<()> {
    fixtures::init_tracing();

    // Setup test dependencies.
    let config = Arc::new(Config::build("test".into()).validate().expect("failed to build Raft config"));
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;

    // Assert all nodes are in non-voter state & have no entries.
    sleep(Duration::from_secs(3)).await;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    sleep(Duration::from_secs(3)).await;
    router.assert_stable_cluster(Some(1), Some(1)).await;

    // Sync some new nodes.
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;
    router.new_raft_node(3).await;
    router.new_raft_node(4).await;
    tracing::info!("--- adding new nodes to cluster");
    let mut new_nodes = futures::stream::FuturesUnordered::new();
    new_nodes.push(router.add_non_voter(0, 1));
    new_nodes.push(router.add_non_voter(0, 2));
    new_nodes.push(router.add_non_voter(0, 3));
    new_nodes.push(router.add_non_voter(0, 4));
    while let Some(inner) = new_nodes.next().await {
        inner?;
    }
    tracing::info!("--- changing cluster config");
    router.change_membership(0, hashset![0, 1, 2, 3, 4]).await?;
    sleep(Duration::from_secs(5)).await;
    router.assert_stable_cluster(Some(1), Some(3)).await; // Still in term 1, so leader is still node 0.

    // Isolate old leader and assert that a new leader takes over.
    tracing::info!("--- isolating master node 0");
    router.isolate_node(0).await;
    sleep(Duration::from_secs(5)).await; // Wait for election and for everything to stabilize (this is way longer than needed).
    router.assert_stable_cluster(Some(2), Some(4)).await;
    let leader = router.leader().await.expect("expected new leader");
    assert!(leader != 0, "expected new leader to be different from the old leader");

    // Restore isolated node.
    router.restore_node(0).await;
    sleep(Duration::from_secs(5)).await; // Wait for election and for everything to stabilize (this is way longer than needed).
    router.assert_stable_cluster(Some(2), Some(4)).await; // We should still be in term 2, as leaders should
                                                          // not be deposed when they are not missing heartbeats.
    let current_leader = router.leader().await.expect("expected to find current leader");
    assert_eq!(leader, current_leader, "expected cluster leadership to stay the same");

    Ok(())
}
