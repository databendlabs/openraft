use std::time::Duration;

use openraft::Config;
use openraft::SnapshotPolicy;

use crate::fixtures::RaftRouter;
use crate::fixtures::expect_timeout;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Call `Raft::trigger_purged()` to purge logs.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn trigger_purge_log() -> anyhow::Result<()> {
    let config = Config {
        enable_heartbeat: false,
        // Disable building snapshot by policy.
        snapshot_policy: SnapshotPolicy::Never,
        // Disable auto purge by policy.
        max_in_snapshot_log_to_keep: u64::MAX,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!(log_index, "--- write some logs");
    {
        let written = router.client_request_many(0, "0", 10).await?;
        let last_log_id = written.last_log_id.expect("nonempty workload");
        log_index = last_log_id.index();

        router.wait_applied_at_least([0, 1, 2], log_index, "write logs").await?;
    }

    tracing::info!(log_index, "--- trigger snapshot for node-0");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().snapshot().await?;

        router.wait(&0).snapshot(log_id(1, 0, log_index), "node-1 snapshot").await?;
    }

    let snapshot_index = log_index;

    tracing::info!(log_index, "--- write another bunch of logs");
    {
        let written = router.client_request_many(0, "0", 10).await?;
        let last_log_id = written.last_log_id.expect("nonempty workload");
        log_index = last_log_id.index();

        router.wait_applied_at_least([0, 1, 2], log_index, "write logs").await?;
    }

    tracing::info!(log_index, "--- purge log for node 0");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().purge_log(snapshot_index).await?;

        router
            .wait(&0)
            .purged(
                Some(log_id(1, 0, snapshot_index)),
                format!("node-0 purged up to {}", snapshot_index),
            )
            .await?;

        n0.trigger().purge_log(log_index).await?;
        let res = router
            .wait_with_timeout(&0, Duration::from_millis(1_000))
            .purged(
                Some(log_id(1, 0, log_index)),
                format!("node-0 wont purged up to {}", log_index),
            )
            .await;

        expect_timeout(res)?;
    }

    Ok(())
}
