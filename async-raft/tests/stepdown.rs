mod fixtures;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_raft::{Config, State};
use maplit::hashset;
use tokio::time::sleep;

use fixtures::RaftRouter;

/// Client write tests.
///
/// What does this test do?
///
/// - create a stable 2-node cluster.
/// - starts a config change which adds two new nodes and removes the leader.
/// - the leader should commit the change to C0 & C1 with separate majorities and then stepdown
///   after the config change is committed.
///
/// RUST_LOG=async_raft,memstore,stepdown=trace cargo test -p async-raft --test stepdown
#[tokio::test(flavor = "multi_thread", worker_threads = 5)]
async fn stepdown() -> Result<()> {
    fixtures::init_tracing();

    // Setup test dependencies.
    let config = Arc::new(Config::build("test".into()).validate().expect("failed to build Raft config"));
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    // Assert all nodes are in non-voter state & have no entries.
    sleep(Duration::from_secs(3)).await;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    sleep(Duration::from_secs(3)).await;
    router.assert_stable_cluster(Some(1), Some(1)).await;

    // Submit a config change which adds two new nodes and removes the current leader.
    let orig_leader = router.leader().await.expect("expected the cluster to have a leader");
    assert_eq!(0, orig_leader, "expected original leader to be node 0");
    router.new_raft_node(2).await;
    router.new_raft_node(3).await;
    router.change_membership(orig_leader, hashset![1, 2, 3]).await?;
    sleep(Duration::from_secs(5)).await; // Give time for step down metrics to flow through.

    // Assert on the state of the old leader.
    {
        let metrics = router
            .latest_metrics()
            .await
            .into_iter()
            .find(|node| node.id == 0)
            .expect("expected to find metrics on original leader node");
        let cfg = metrics.membership_config;
        assert!(metrics.state != State::Leader, "expected old leader to have stepped down");
        assert_eq!(
            metrics.current_term, 1,
            "expected old leader to still be in first term, got {}",
            metrics.current_term
        );
        assert_eq!(
            metrics.last_log_index, 3,
            "expected old leader to have last log index of 3, got {}",
            metrics.last_log_index
        );
        assert_eq!(
            metrics.last_applied, 3,
            "expected old leader to have last applied of 3, got {}",
            metrics.last_applied
        );
        assert_eq!(
            cfg.members,
            hashset![1, 2, 3],
            "expected old leader to have membership of [1, 2, 3], got {:?}",
            cfg.members
        );
        assert!(cfg.members_after_consensus.is_none(), "expected old leader to be out of joint consensus");
    }

    // Assert that the current cluster is stable.
    let _ = router.remove_node(0).await;
    sleep(Duration::from_secs(5)).await; // Give time for a new leader to be elected.
    router.assert_stable_cluster(Some(2), Some(4)).await;
    router.assert_storage_state(2, 4, None, 0, None).await;
    // ----------------------------------- ^^^ this is `0` instead of `4` because blank payloads from new leaders
    //                                         and config change entries are never applied to the state machine.

    Ok(())
}
