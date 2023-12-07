use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::LogId;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Manually trigger a snapshot with `Raft::trigger_snapshot()` on Leader and Follower.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn trigger_snapshot() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1}, btreeset! {}).await?;

    tracing::info!(log_index, "--- trigger snapshot for node-1");
    {
        let n1 = router.get_raft_handle(&1)?;
        n1.trigger().snapshot().await?;

        router
            .wait(&1, timeout())
            .snapshot(LogId::new(CommittedLeaderId::new(1, 0), log_index), "node-1 snapshot")
            .await?;
    }

    tracing::info!(log_index, "--- send some logs");
    {
        router.client_request_many(0, "0", 10).await?;
        log_index += 10;

        router.wait(&0, timeout()).applied_index(Some(log_index), "node-0 write logs").await?;
        router.wait(&1, timeout()).applied_index(Some(log_index), "node-1 write logs").await?;
    }

    tracing::info!(log_index, "--- trigger snapshot for node-0");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().snapshot().await?;

        router
            .wait(&0, timeout())
            .snapshot(LogId::new(CommittedLeaderId::new(1, 0), log_index), "node-0 snapshot")
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
