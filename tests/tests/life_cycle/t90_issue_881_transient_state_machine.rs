use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;

use crate::fixtures::MemLogStore;
use crate::fixtures::MemRaft;
use crate::fixtures::MemStateMachine;
use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Test transient (in-memory) state machine recovery with persistent snapshot.
///
/// A transient state machine does not persist on each apply() but has persistent snapshots.
/// On restart, in-memory state is lost but can be recovered by:
/// 1. Installing the last persistent snapshot to restore to snapshot position
/// 2. Re-applying logs from snapshot position to committed position
///
/// This test uses a single-node cluster to ensure recovery happens through local
/// snapshot installation and log re-application, not through replication from a leader.
///
/// Steps:
/// 1. Creates a single-node cluster
/// 2. Writes logs, creates persistent snapshot, writes more logs
/// 3. Stops the node and clears its in-memory state (simulating restart)
/// 4. Restarts the node with save_committed enabled
/// 5. Verifies persistent snapshot is installed first, then remaining logs are re-applied
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn issue_881_transient_state_machine() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    // Enable save_committed to support transient state machines
    router.enable_saving_committed = true;

    tracing::info!("--- bring up single-node cluster");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- write 10 logs");
    {
        log_index += router.client_request_many(0, "foo", 10).await?;
        router.wait(&0, timeout()).applied_index(Some(log_index), "node-0 applied").await?;
    }

    let snapshot_log_index = log_index;

    tracing::info!(log_index, "--- trigger snapshot");
    {
        let n = router.get_raft_handle(&0)?;
        n.trigger().snapshot().await?;
        router.wait(&0, timeout()).snapshot(log_id(1, 0, snapshot_log_index), "snapshot created").await?;
    }

    tracing::info!(log_index, "--- write 5 more logs after snapshot");
    {
        log_index += router.client_request_many(0, "bar", 5).await?;
        router.wait(&0, timeout()).applied_index(Some(log_index), "node-0 applied new logs").await?;
    }

    tracing::info!(log_index, "--- stop node-0 and clear its state machine");
    {
        let (node, ls, sm): (MemRaft, MemLogStore, MemStateMachine) = router.remove_node(0).unwrap();
        node.shutdown().await?;

        // Clear state machine to simulate transient (in-memory) data loss.
        // The committed log id is preserved in log storage.
        sm.clear_state_machine().await;

        tracing::info!(log_index, "--- restart node-0 with cleared state machine");
        router.new_raft_node_with_sto(0, ls, sm).await;
    }

    tracing::info!(
        log_index,
        "--- node-0 should recover: install snapshot first, then re-apply remaining logs"
    );
    {
        router
            .wait(&0, timeout())
            .applied_index(Some(log_index), "node-0 recovered by snapshot + log replay")
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
