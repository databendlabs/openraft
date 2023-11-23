use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::LogId;
use openraft::SnapshotPolicy;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Test replication when switching line rate to snapshotting.
///
/// What does this test do?
///
/// - bring on a cluster of 1 voter and 1 learner.
/// - send several logs and check the replication.
/// - isolate replication to node 1. send some other logs to trigger snapshot on node 0. The logs on
///   node 0 should be removed.
/// - restore replication.
/// - ensure that replication is switched from line-rate mode to snapshotting mode, on absence of
///   logs.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn snapshot_line_rate_to_snapshot() -> Result<()> {
    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {1}).await?;

    tracing::info!(log_index, "--- send more than half threshold logs");
    {
        router.client_request_many(0, "0", (snapshot_threshold / 2 + 2 - log_index) as usize).await?;
        log_index = snapshot_threshold / 2 + 2;

        router
            .wait_for_log(
                &btreeset![0, 1],
                Some(log_index),
                timeout(),
                "send log to trigger snapshot",
            )
            .await?;
    }

    tracing::info!(log_index, "--- stop replication to node 1");
    tracing::info!(log_index, "--- send just enough logs to trigger snapshot");
    {
        router.set_network_error(1, true);

        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await?;
        log_index = snapshot_threshold - 1;

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
                LogId::new(CommittedLeaderId::new(1, 0), log_index),
                timeout(),
                "snapshot on node 0",
            )
            .await?;
    }

    tracing::info!(log_index, "--- restore node 1 and replication");
    {
        router.set_network_error(1, false);

        router.wait_for_log(&btreeset![1], Some(log_index), timeout(), "replicate by snapshot").await?;
        router
            .wait_for_snapshot(
                &btreeset![1],
                LogId::new(CommittedLeaderId::new(1, 0), log_index),
                timeout(),
                "snapshot on node 1",
            )
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
