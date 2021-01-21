mod fixtures;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_raft::raft::MembershipConfig;
use async_raft::{Config, SnapshotPolicy};
use maplit::hashset;
use tokio::time::sleep;

use fixtures::RaftRouter;

/// Compaction test.
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - add new nodes and assert that they receive the snapshot.
///
/// RUST_LOG=async_raft,memstore,compaction=trace cargo test -p async-raft --test compaction
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn compaction() -> Result<()> {
    fixtures::init_tracing();

    // Setup test dependencies.
    let config = Arc::new(
        Config::build("test".into())
            .snapshot_policy(SnapshotPolicy::LogsSinceLast(500))
            .validate()
            .expect("failed to build Raft config"),
    );
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;

    // Assert all nodes are in non-voter state & have no entries.
    sleep(Duration::from_secs(10)).await;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    sleep(Duration::from_secs(10)).await;
    router.assert_stable_cluster(Some(1), Some(1)).await;

    // Send enough requests to the cluster that compaction on the node should be triggered.
    router.client_request_many(0, "0", 499).await; // Puts us exactly at the configured snapshot policy threshold.
    sleep(Duration::from_secs(5)).await; // Wait to ensure there is enough time for a snapshot to be built (this is way more than enough).
    router.assert_stable_cluster(Some(1), Some(500)).await;
    router
        .assert_storage_state(
            1,
            500,
            Some(0),
            500,
            Some((
                500.into(),
                1,
                MembershipConfig {
                    members: hashset![0],
                    members_after_consensus: None,
                },
            )),
        )
        .await;

    // Add a new node and assert that it received the same snapshot.
    router.new_raft_node(1).await;
    router.add_non_voter(0, 1).await.expect("failed to add new node as non-voter");
    router
        .change_membership(0, hashset![0, 1])
        .await
        .expect("failed to modify cluster membership");
    sleep(Duration::from_secs(5)).await; // Wait to ensure metrics are updated (this is way more than enough).
    router.assert_stable_cluster(Some(1), Some(502)).await; // We expect index to be 500 + 2 (joint & uniform config change entries).
    let expected_snap = Some((
        500.into(),
        1,
        MembershipConfig {
            members: hashset![0u64],
            members_after_consensus: None,
        },
    ));
    router.assert_storage_state(1, 502, None, 500, expected_snap).await;
    // -------------------------------- ^^^^ this value is None because non-voters do not vote.

    Ok(())
}
