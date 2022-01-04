use std::sync::Arc;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::Config;
use openraft::LogId;
use openraft::SnapshotPolicy;
use openraft::State;

#[macro_use]
mod fixtures;

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
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_ge_half_threshold() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

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
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut n_logs = 0;

    tracing::info!("--- initializing cluster");
    {
        router.new_raft_node(0).await;

        router.wait_for_log(&btreeset![0], n_logs, None, "empty").await?;
        router.wait_for_state(&btreeset![0], State::Learner, None, "empty").await?;
        router.initialize_from_single_node(0).await?;
        n_logs += 1;

        router.wait_for_log(&btreeset![0], n_logs, None, "init leader").await?;
        router.assert_stable_cluster(Some(1), Some(n_logs)).await;
    }

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - n_logs) as usize).await;
        n_logs = snapshot_threshold;

        router.wait_for_log(&btreeset![0], n_logs, None, "send log to trigger snapshot").await?;
        router.assert_stable_cluster(Some(1), Some(n_logs)).await;

        router.wait_for_snapshot(&btreeset![0], LogId { term: 1, index: n_logs }, None, "snapshot").await?;
        router
            .assert_storage_state(
                1,
                n_logs,
                Some(0),
                LogId { term: 1, index: n_logs },
                Some((n_logs.into(), 1)),
            )
            .await?;
    }

    tracing::info!("--- send logs to make distance between snapshot index and last_log_index");
    {
        router.client_request_many(0, "0", (log_cnt - n_logs) as usize).await;
        n_logs = log_cnt;
    }

    tracing::info!("--- add learner to receive snapshot and logs");
    {
        router.new_raft_node(1).await;
        router.add_learner(0, 1).await.expect("failed to add new node as learner");

        router.wait_for_log(&btreeset![0, 1], n_logs, None, "add learner").await?;
        let expected_snap = Some((n_logs.into(), 1));
        router.wait_for_snapshot(&btreeset![1], LogId { term: 1, index: n_logs }, None, "").await?;
        router
            .assert_storage_state(
                1,
                n_logs,
                None, /* learner does not vote */
                LogId { term: 1, index: n_logs },
                expected_snap,
            )
            .await?;
    }

    Ok(())
}
