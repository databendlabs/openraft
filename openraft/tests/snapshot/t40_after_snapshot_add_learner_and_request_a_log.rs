use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::SnapshotPolicy;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Test log replication after snapshot replication.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn after_snapshot_add_learner_and_request_a_log() -> Result<()> {
    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            max_applied_log_to_keep: 2, // do not let add-learner log and client-write log to trigger a snapshot.
            purge_batch_size: 1,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;
    let snapshot_index;

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await?;
        log_index = snapshot_threshold - 1;
        snapshot_index = log_index;

        router
            .wait_for_log(
                &btreeset![0],
                Some(log_index),
                timeout(),
                "send log to trigger snapshot",
            )
            .await?;
        router.assert_stable_cluster(Some(1), Some(log_index));

        router
            .wait(&0, timeout())
            .snapshot(
                LogId::new(LeaderId::new(1, 0), snapshot_index),
                "leader-0 has built snapshot",
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

    tracing::info!("--- add learner to the cluster to receive snapshot, which overrides the learner storage");
    {
        router.new_raft_node(1);
        router.add_learner(0, 1).await.expect("failed to add new node as learner");
        log_index += 1;

        tracing::info!("--- DONE add learner");

        router
            .wait(&1, timeout())
            .snapshot(
                LogId::new(LeaderId::new(1, 0), snapshot_index),
                "learner-1 receives snapshot",
            )
            .await?;

        log_index += router.client_request_many(0, "0", 1).await?;
        tracing::info!("--- after request a log");

        router
            .wait_for_log(
                &btreeset![0, 1],
                Some(log_index),
                timeout(),
                "learner-1 receives client-write log",
            )
            .await?;

        let expected_snap = Some((snapshot_index.into(), 1));

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

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
