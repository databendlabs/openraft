use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::storage::RaftLogStorage;
use openraft::Config;
use openraft::SnapshotPolicy;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::log_id;
use crate::fixtures::RaftRouter;

/// When startup, if there is no snapshot and there are logs purged, it should build a snapshot at
/// once.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn startup_build_snapshot() -> anyhow::Result<()> {
    let snapshot_threshold = 10;

    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            max_in_snapshot_log_to_keep: 0,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- send client requests");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await?;
        log_index = snapshot_threshold - 1;
    }

    tracing::info!("--- shut down and purge to log index: {}", 5);
    let (_, mut log_store, sm) = router.remove_node(0).unwrap();
    log_store.purge(log_id(1, 0, 5)).await?;

    tracing::info!("--- restart, expect snapshot at index: {} for node-1", log_index);
    {
        router.new_raft_node_with_sto(0, log_store, sm).await;
        router.wait(&0, timeout()).snapshot(log_id(1, 0, log_index), "node-1 snapshot").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
