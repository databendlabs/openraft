use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LogId;
use openraft::SnapshotPolicy;

use crate::fixtures::RaftRouter;

/// Send a command to raft to trigger snapshot replication on leader. The leader should send a snapshot at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 10)]
async fn trigger_snapshot() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let snapshot_threshold: u64 = 1000;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            max_applied_log_to_keep: 1000,
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut log_index = router.new_nodes_from_single(btreeset! {0,1,2}, btreeset! {3}).await?;

    tracing::info!("--- send several logs");
    {
        router.client_request_many(0, "0", 10).await;
        log_index += 10;

        router.wait_for_log(&btreeset![0, 1, 2, 3], Some(log_index), timeout(), "send log").await?;

        for i in [0, 1, 2, 3] {
            tracing::info!("--- waiting for non-snapshot for node {}", i);
            router.wait(&i, timeout()).await?.metrics(|x| x.snapshot.is_none(), "snapshot is not built").await?;
        }
    }

    tracing::info!("--- trigger snapshot and check");
    {
        let leader = router.get_raft_handle(&0).await?;
        leader.trigger_snapshot().await?;

        for i in [0, 1, 2, 3] {
            tracing::info!("--- waiting for snapshot for node {}", i);
            router
                .wait(&i, timeout())
                .await?
                .snapshot(
                    LogId {
                        term: 1,
                        index: log_index,
                    },
                    "snapshot is built and replicated",
                )
                .await?;
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
