use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::MemLogStore;
use crate::fixtures::MemRaft;
use crate::fixtures::MemStateMachine;
use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// A single leader should re-apply all logs upon startup,
/// because itself is a quorum.
///
/// This test disables save_committed() to ensure that logs are still re-applied because the leader
/// itself forms a quorum.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn single_leader_restart_re_apply_logs() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.enable_saving_committed = false;

    tracing::info!("--- bring up cluster of 1 node");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- write to 1 log");
    {
        log_index += router.client_request_many(0, "foo", 1).await?;
    }

    tracing::info!(log_index, "--- stop and restart node-0");
    {
        let (node, ls, sm): (MemRaft, MemLogStore, MemStateMachine) = router.remove_node(0).unwrap();
        node.shutdown().await?;

        // Clear state machine, logs should be re-applied upon restart, because it is a leader.
        sm.clear_state_machine().await;

        tracing::info!(log_index, "--- restart node-0");

        router.new_raft_node_with_sto(0, ls, sm).await;
        router.wait(&0, timeout()).state(ServerState::Leader, "become leader upon restart").await?;
    }

    tracing::info!(log_index, "--- a single leader should re-apply all logs");
    {
        router.wait(&0, timeout()).applied_index(Some(log_index), "node-0 works").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
