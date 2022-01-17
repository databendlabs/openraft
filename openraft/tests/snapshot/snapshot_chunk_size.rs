use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LogId;
use openraft::SnapshotPolicy;
use openraft::State;

use crate::fixtures::RaftRouter;

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

    let mut log_index = 0;

    tracing::info!("--- initializing cluster");
    {
        router.new_raft_node(0).await;

        router.wait_for_log(&btreeset![0], None, timeout(), "empty").await?;
        router.wait_for_state(&btreeset![0], State::Learner, timeout(), "empty").await?;

        router.initialize_from_single_node(0).await?;
        log_index += 1;

        router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "init leader").await?;
    }

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await;
        log_index = snapshot_threshold - 1;

        let want_snap = Some((log_index.into(), 1));

        router
            .wait_for_log(
                &btreeset![0],
                Some(log_index),
                timeout(),
                "send log to trigger snapshot",
            )
            .await?;
        router
            .wait_for_snapshot(
                &btreeset![0],
                LogId {
                    term: 1,
                    index: log_index,
                },
                None,
                "snapshot",
            )
            .await?;
        router
            .assert_storage_state(
                1,
                log_index,
                Some(0),
                LogId {
                    term: 1,
                    index: log_index,
                },
                want_snap,
            )
            .await?;
    }

    tracing::info!("--- add learner to receive snapshot and logs");
    {
        router.new_raft_node(1).await;
        router.add_learner(0, 1).await.expect("failed to add new node as learner");
        n_logs += 1;

        router.wait_for_log(&btreeset![0, 1], n_logs, None, "add learner").await?;
        router.wait_for_snapshot(&btreeset![1], LogId { term: 1, index: n_logs }, None, "").await?;

        router
            .assert_storage_state(
                1,
                n_logs,
                Some(0), /* leader vote for self */
                LogId { term: 1, index: n_logs },
                Some(((n_logs).into(), 1)),
            )
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
