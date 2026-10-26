use openraft::Config;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftStateMachine;

use crate::fixtures::Node;
use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// When startup, if there is no snapshot and there are logs purged, it should build a snapshot at
/// once.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn startup_build_snapshot() -> anyhow::Result<()> {
    let config = Config {
        enable_heartbeat: false,
        max_in_snapshot_log_to_keep: 0,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster([0], []).await?;

    tracing::info!(log_index, "--- send client requests");
    {
        let written = router.client_request_many(0, "0", (20 - 1 - log_index) as usize).await?;
        log_index += written.count;

        router.wait(&0).applied_index(Some(log_index), "node-0 applied all requests").await?;
        router.get_raft_handle(&0)?.trigger().snapshot().await?;
        router.wait(&0).snapshot(log_id(1, 0, log_index), "node-0 snapshot").await?;
    }

    tracing::info!(log_index, "--- shut down and purge to log index: {}", 5);
    let Node {
        mut log_store,
        state_machine: mut sm,
        ..
    } = router.stop_node(0).await?;
    log_store.purge(log_id(1, 0, 19)).await?;

    tracing::info!(log_index, "--- drop current snapshot");
    {
        sm.drop_snapshot().await;
        let snap = sm.get_current_snapshot().await?;
        assert!(snap.is_none());
    }

    tracing::info!(
        log_index,
        "--- restart, expect snapshot at index: {} for node-1",
        log_index
    );
    {
        router.new_raft_node_with_sto(0, log_store, sm).await?;
        router.wait(&0).snapshot(log_id(1, 0, log_index), "node-1 snapshot").await?;
    }

    Ok(())
}
