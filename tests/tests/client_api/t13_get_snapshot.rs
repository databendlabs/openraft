use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::testing::log_id;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Get snapshot with `Raft::get_snapshot()`
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn get_snapshot() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1}, btreeset! {}).await?;

    tracing::info!(log_index, "--- get None snapshot for node-1");
    {
        let n1 = router.get_raft_handle(&1)?;

        let curr_snap = n1.get_snapshot().await?;
        assert!(curr_snap.is_none());
    }

    tracing::info!(log_index, "--- trigger and get snapshot for node-1");
    {
        let n1 = router.get_raft_handle(&1)?;
        n1.trigger().snapshot().await?;

        router.wait(&1, timeout()).snapshot(log_id(1, 0, log_index), "node-1 snapshot").await?;

        let curr_snap = n1.get_snapshot().await?;
        let snap = curr_snap.unwrap();
        assert_eq!(snap.meta.last_log_id, Some(log_id(1, 0, log_index)));
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
