use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::testing::log_id;
use openraft::Config;
use openraft::SnapshotPolicy;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Call `Raft::trigger_purged()` to purge logs.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn trigger_purge_log() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            // Disable building snapshot by policy.
            snapshot_policy: SnapshotPolicy::Never,
            // Disable auto purge by policy.
            max_in_snapshot_log_to_keep: u64::MAX,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- write some logs");
    {
        log_index += router.client_request_many(0, "0", 10).await?;

        for id in [0, 1, 2] {
            router
                .wait(&id, timeout())
                .applied_index(Some(log_index), format_args!("node-{} write logs", id))
                .await?;
        }
    }

    tracing::info!(log_index, "--- trigger snapshot for node-0");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().snapshot().await?;

        router.wait(&0, timeout()).snapshot(log_id(1, 0, log_index), "node-1 snapshot").await?;
    }

    let snapshot_index = log_index;

    tracing::info!(log_index, "--- write another bunch of logs");
    {
        log_index += router.client_request_many(0, "0", 10).await?;

        for id in [0, 1, 2] {
            router
                .wait(&id, timeout())
                .applied_index(Some(log_index), format_args!("node-{} write logs", id))
                .await?;
        }
    }

    tracing::info!(log_index, "--- purge log for node 0");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().purge_log(snapshot_index).await?;

        router
            .wait(&0, timeout())
            .purged(
                Some(log_id(1, 0, snapshot_index)),
                format_args!("node-0 purged upto {}", snapshot_index),
            )
            .await?;

        n0.trigger().purge_log(log_index).await?;
        let res = router
            .wait(&0, timeout())
            .purged(
                Some(log_id(1, 0, log_index)),
                format_args!("node-0 wont purged upto {}", log_index),
            )
            .await;

        assert!(res.is_err(), "can not purge logs not in snapshot");
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
