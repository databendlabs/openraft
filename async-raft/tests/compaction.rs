use std::sync::Arc;

use anyhow::Result;
use async_raft::raft::MembershipConfig;
use async_raft::Config;
use async_raft::LogId;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::btreeset;

#[macro_use]
mod fixtures;

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
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let snapshot_threshold: u64 = 50;

    // Setup test dependencies.
    let config =
        Arc::new(Config::build(&["foo", "--snapshot-policy", "since_last:50"]).expect("failed to build Raft config"));
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;

    let mut want = 0;

    // Assert all nodes are in non-voter state & have no entries.
    router.wait_for_log(&btreeset![0], want, None, "empty").await?;
    router.wait_for_state(&btreeset![0], State::NonVoter, None, "empty").await?;

    router.assert_pristine_cluster().await;

    tracing::info!("--- initializing cluster");

    router.initialize_from_single_node(0).await?;
    want += 1;

    router.wait_for_log(&btreeset![0], want, None, "init leader").await?;
    router.assert_stable_cluster(Some(1), Some(1)).await;

    // Send enough requests to the cluster that compaction on the node should be triggered.
    // Puts us exactly at the configured snapshot policy threshold.
    router.client_request_many(0, "0", (snapshot_threshold - want) as usize).await;
    want = snapshot_threshold;

    router.wait_for_log(&btreeset![0], want, None, "write").await?;
    router.assert_stable_cluster(Some(1), Some(want)).await;
    router.wait_for_snapshot(&btreeset![0], LogId { term: 1, index: want }, None, "snapshot").await?;
    router
        .assert_storage_state(
            1,
            want,
            Some(0),
            LogId { term: 1, index: want },
            Some((want.into(), 1, MembershipConfig {
                members: btreeset![0],
                members_after_consensus: None,
            })),
        )
        .await;

    // Add a new node and assert that it received the same snapshot.
    router.new_raft_node(1).await;
    router.add_non_voter(0, 1).await.expect("failed to add new node as non-voter");

    tracing::info!("--- add 1 log after snapshot");

    router.client_request_many(0, "0", 1).await;
    want += 1;

    router.wait_for_log(&btreeset![0, 1], want, None, "add follower").await?;
    let expected_snap = Some((snapshot_threshold.into(), 1, MembershipConfig {
        members: btreeset![0u64],
        members_after_consensus: None,
    }));
    router
        .assert_storage_state(
            1,
            want,
            None, /* non-voter does not vote */
            LogId { term: 1, index: want },
            expected_snap,
        )
        .await;

    Ok(())
}
