use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::SnapshotPolicy;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// A leader should create and send snapshot when snapshot is old and is not that old to trigger a snapshot, i.e.:
/// `threshold/2 < leader.last_log_index - snapshot.applied_index < threshold`
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - send some other log after snapshot created, to make the `leader.last_log_index - snapshot.applied_index` big
///   enough.
/// - add learner and assert that they receive the snapshot and logs.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn snapshot_ge_half_threshold() -> Result<()> {
    let snapshot_threshold: u64 = 10;
    let log_cnt = snapshot_threshold + 6;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            max_applied_log_to_keep: 6,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await;
        log_index = snapshot_threshold - 1;

        router.wait_for_log(&btreeset![0], Some(log_index), None, "send log to trigger snapshot").await?;
        router.assert_stable_cluster(Some(1), Some(log_index)).await;

        router
            .wait_for_snapshot(
                &btreeset![0],
                LogId::new(LeaderId::new(1, 0), log_index),
                None,
                "snapshot",
            )
            .await?;
        router
            .assert_storage_state(
                1,
                log_index,
                Some(0),
                LogId::new(LeaderId::new(1, 0), log_index),
                Some((log_index.into(), 1)),
            )
            .await?;
    }

    tracing::info!("--- send logs to make distance between snapshot index and last_log_index");
    {
        router.client_request_many(0, "0", (log_cnt - log_index) as usize).await;
        log_index = log_cnt;
    }

    tracing::info!("--- add learner to receive snapshot and logs");
    {
        router.new_raft_node(1).await;
        router.add_learner(0, 1).await.expect("failed to add new node as learner");
        log_index += 1;

        router.wait_for_log(&btreeset![0, 1], Some(log_index), None, "add learner").await?;
        let expected_snap = Some((log_index.into(), 1));
        router
            .wait_for_snapshot(&btreeset![1], LogId::new(LeaderId::new(1, 0), log_index), None, "")
            .await?;
        router
            .assert_storage_state(
                1,
                log_index,
                None, /* learner does not vote */
                LogId::new(LeaderId::new(1, 0), log_index),
                expected_snap,
            )
            .await?;
    }

    Ok(())
}
