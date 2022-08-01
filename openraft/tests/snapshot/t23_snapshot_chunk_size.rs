use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::ServerState;
use openraft::SnapshotPolicy;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Test transfer snapshot in small chnuks
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - add learner and assert that they receive the snapshot and logs.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn snapshot_chunk_size() -> Result<()> {
    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            snapshot_max_chunk_size: 10,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = 0;

    tracing::info!("--- initializing cluster");
    {
        router.new_raft_node(0);

        router.wait_for_log(&btreeset![0], None, timeout(), "empty").await?;
        router.wait_for_state(&btreeset![0], ServerState::Learner, timeout(), "empty").await?;

        router.initialize_from_single_node(0).await?;
        log_index += 1;

        router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "init leader").await?;
    }

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await?;
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
                want_snap,
            )
            .await?;
    }

    tracing::info!("--- add learner to receive snapshot and logs");
    {
        router.new_raft_node(1);
        router.add_learner(0, 1).await.expect("failed to add new node as learner");
        log_index += 1;

        router.wait_for_log(&btreeset![0, 1], Some(log_index), None, "add learner").await?;
        router
            .wait_for_snapshot(&btreeset![1], LogId::new(LeaderId::new(1, 0), log_index), None, "")
            .await?;

        router
            .wait_for_snapshot(&btreeset![0], LogId::new(LeaderId::new(1, 0), log_index - 1), None, "")
            .await?;

        // after add_learner, log_index + 1,
        // leader has only log_index log in snapshot, cause it has compacted before add_learner
        let mut store = router.get_storage_handle(&0)?;
        router
            .assert_storage_state_with_sto(
                &mut store,
                &0,
                1,
                log_index,
                Some(0),
                LogId::new(LeaderId::new(1, 0), log_index),
                &Some(((log_index - 1).into(), 1)),
            )
            .await?;

        // learner has log_index + 1 log in snapshot, cause it do compact after add_learner,
        // so learner's snapshot include add_learner log
        let mut store = router.get_storage_handle(&1)?;
        router
            .assert_storage_state_with_sto(
                &mut store,
                &1,
                1,
                log_index,
                Some(0),
                LogId::new(LeaderId::new(1, 0), log_index),
                &Some(((log_index).into(), 1)),
            )
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
