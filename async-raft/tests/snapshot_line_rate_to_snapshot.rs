use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_raft::Config;
use async_raft::LogId;
use fixtures::RaftRouter;
use maplit::btreeset;

#[macro_use]
mod fixtures;

/// Test replication when switching line rate to snapshotting.
///
/// What does this test do?
///
/// - bring on a cluster of 1 voter and 1 non-voter.
/// - send several logs and check the replication.
/// - isolate replication to node 1. send some other logs to trigger snapshot on node 0. The logs on node 0 should be
///   removed.
/// - restore replication.
/// - ensure that replication is switched from line-rate mode to snapshotting mode, on absence of logs.
///
/// export RUST_LOG=async_raft,memstore,snapshot_line_rate_to_snapshot=trace
/// cargo test -p async-raft --test snapshot_line_rate_to_snapshot
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_line_rate_to_snapshot() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let snapshot_threshold: u64 = 10;

    let config =
        Arc::new(Config::build(&["foo", "--snapshot-policy", "since_last:10"]).expect("failed to build Raft config"));
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut n_logs = router.new_nodes_from_single(btreeset! {0}, btreeset! {1}).await?;

    tracing::info!("--- send more than half threshold logs");
    {
        router.client_request_many(0, "0", (snapshot_threshold / 2 + 2 - n_logs) as usize).await;
        n_logs = snapshot_threshold / 2 + 2;

        router.wait_for_log(&btreeset![0, 1], n_logs, timeout(), "send log to trigger snapshot").await?;
    }

    tracing::info!("--- stop replication to node 1");
    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.isolate_node(1).await;

        router.client_request_many(0, "0", (snapshot_threshold - n_logs) as usize).await;

        n_logs = snapshot_threshold;

        router.wait_for_log(&btreeset![0], n_logs, timeout(), "send log to trigger snapshot").await?;
        router
            .wait_for_snapshot(
                &btreeset![0],
                LogId { term: 1, index: n_logs },
                timeout(),
                "snapshot on node 0",
            )
            .await?;
    }

    tracing::info!("--- restore node 1 and replication");
    {
        router.restore_node(1).await;

        router.wait_for_log(&btreeset![1], n_logs, timeout(), "replicate by snapshot").await?;
        router
            .wait_for_snapshot(
                &btreeset![1],
                LogId { term: 1, index: n_logs },
                timeout(),
                "snapshot on node 1",
            )
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
