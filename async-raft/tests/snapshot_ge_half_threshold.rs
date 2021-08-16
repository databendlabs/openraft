mod fixtures;

use std::sync::Arc;

use anyhow::Result;
use async_raft::raft::MembershipConfig;
use async_raft::Config;
use async_raft::LogId;
use async_raft::SnapshotPolicy;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::btreeset;

/// A leader should create and send snapshot when snapshot is old and is not that old to trigger a snapshot, i.e.:
/// `threshold/2 < leader.last_log_index - snapshot.applied_index < threshold`
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - send some other log after snapshot created, to make the `leader.last_log_index - snapshot.applied_index` big
///   enough.
/// - add non-voter and assert that they receive the snapshot and logs.
///
/// export RUST_LOG=async_raft,memstore,snapshot_ge_half_threshold=trace
/// cargo test -p async-raft --test snapshot_ge_half_threshold
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_ge_half_threshold() -> Result<()> {
    fixtures::init_tracing();

    let snapshot_threshold: u64 = 10;
    let log_cnt = snapshot_threshold + 6;

    let config = Arc::new(
        Config::build("test".into())
            .snapshot_policy(SnapshotPolicy::LogsSinceLast(snapshot_threshold))
            .validate()
            .expect("failed to build Raft config"),
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut want = 0;

    tracing::info!("--- initializing cluster");
    {
        router.new_raft_node(0).await;

        router.wait_for_log(&btreeset![0], want, None, "empty").await?;
        router.wait_for_state(&btreeset![0], State::NonVoter, None, "empty").await?;
        router.initialize_from_single_node(0).await?;
        want += 1;

        router.wait_for_log(&btreeset![0], want, None, "init leader").await?;
        router.assert_stable_cluster(Some(1), Some(want)).await;
    }

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - want) as usize).await;
        want = snapshot_threshold;

        router.wait_for_log(&btreeset![0], want, None, "send log to trigger snapshot").await?;
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
    }

    tracing::info!("--- send logs to make distance between snapshot index and last_log_index");
    {
        router.client_request_many(0, "0", (log_cnt - want) as usize).await;
        want = log_cnt;
    }

    tracing::info!("--- add non-voter to receive snapshot and logs");
    {
        router.new_raft_node(1).await;
        router.add_non_voter(0, 1).await.expect("failed to add new node as non-voter");

        router.wait_for_log(&btreeset![0, 1], want, None, "add non-voter").await?;
        let expected_snap = Some((want.into(), 1, MembershipConfig {
            members: btreeset![0u64],
            members_after_consensus: None,
        }));
        router.wait_for_snapshot(&btreeset![1], LogId { term: 1, index: want }, None, "").await?;
        router
            .assert_storage_state(
                1,
                want,
                None, /* non-voter does not vote */
                LogId { term: 1, index: want },
                expected_snap,
            )
            .await;
    }

    Ok(())
}
