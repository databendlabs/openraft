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

/// Test transfer snapshot in small chnuks
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - add learner and assert that they receive the snapshot and logs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_chunk_size() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            snapshot_max_chunk_size: 10,
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
    }

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - n_logs) as usize).await;
        n_logs = snapshot_threshold;

        let want_snap = Some((n_logs.into(), 1));

        router.wait_for_log(&btreeset![0], n_logs, None, "send log to trigger snapshot").await?;
        router.wait_for_snapshot(&btreeset![0], LogId { term: 1, index: n_logs }, None, "snapshot").await?;
        router.assert_storage_state(1, n_logs, Some(0), LogId { term: 1, index: n_logs }, want_snap).await?;
    }

    tracing::info!("--- add learner to receive snapshot and logs");
    {
        router.new_raft_node(1).await;
        router.add_learner(0, 1).await.expect("failed to add new node as learner");

        let want_snap = Some((n_logs.into(), 1));

        router.wait_for_log(&btreeset![0, 1], n_logs, None, "add learner").await?;
        router.wait_for_snapshot(&btreeset![1], LogId { term: 1, index: n_logs }, None, "").await?;
        router
            .assert_storage_state(
                1,
                n_logs,
                None, /* learner does not vote */
                LogId { term: 1, index: n_logs },
                want_snap,
            )
            .await?;
    }

    Ok(())
}
