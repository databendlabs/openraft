use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::ServerState;
use openraft::SnapshotPolicy;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Test transfer snapshot in small chunks
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - add learner and assert that they receive the snapshot and logs.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_chunk_size() -> Result<()> {
    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            snapshot_max_chunk_size: 10,
            enable_heartbeat: false,
            max_in_snapshot_log_to_keep: 0,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = 0;

    tracing::info!(log_index, "--- initializing cluster");
    {
        router.new_raft_node(0).await;

        router.wait(&0, timeout()).applied_index(None, "empty").await?;
        router.wait(&0, timeout()).state(ServerState::Learner, "empty").await?;

        router.initialize(0).await?;
        log_index += 1;

        router.wait(&0, timeout()).applied_index(Some(log_index), "init leader").await?;
    }

    tracing::info!(log_index, "--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await?;
        log_index = snapshot_threshold - 1;

        let want_snap = Some((log_index.into(), 1));

        router.wait(&0, timeout()).applied_index(Some(log_index), "send log to trigger snapshot").await?;
        router.wait(&0, timeout()).snapshot(log_id(1, 0, log_index), "snapshot").await?;
        router.assert_storage_state(1, log_index, Some(0), log_id(1, 0, log_index), want_snap).await?;

        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().purge_log(log_index).await?;
        router
            .wait(&0, timeout())
            .purged(Some(log_id(1, 0, 9)), "purge Leader-0 all in snapshot logs")
            .await?;
    }

    tracing::info!(log_index, "--- add learner to receive snapshot and logs");
    {
        router.new_raft_node(1).await;
        router.add_learner(0, 1).await.expect("failed to add new node as learner");
        log_index += 1;

        for id in [0, 1] {
            router.wait(&id, timeout()).applied_index(Some(log_index), "add learner").await?;
        }
        router.wait(&1, timeout()).applied_index(Some(log_index), "sync all data to learner-1").await?;
        router.wait(&1, timeout()).snapshot(log_id(1, 0, log_index - 1), "learner-1 snapshot").await?;

        // after add_learner, log_index + 1,
        // leader has only log_index log in snapshot, cause it has compacted before add_learner
        let (mut store, mut sm) = router.get_storage_handle(&0)?;
        router
            .assert_storage_state_with_sto(
                &mut store,
                &mut sm,
                &0,
                1,
                log_index,
                Some(0),
                log_id(1, 0, log_index),
                &Some(((log_index - 1).into(), 1)),
            )
            .await?;

        let (mut store, mut sm) = router.get_storage_handle(&1)?;
        router
            .assert_storage_state_with_sto(
                &mut store,
                &mut sm,
                &1,
                1,
                log_index,
                Some(0),
                log_id(1, 0, log_index),
                &Some(((log_index - 1).into(), 1)),
            )
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
