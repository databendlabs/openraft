use std::sync::Arc;
use std::time::Duration;

use maplit::btreemap;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::MemLogStore;
use crate::fixtures::MemRaft;
use crate::fixtures::MemStateMachine;
use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Test leader restart with complete state loss and recovery.
///
/// A 3-nodes cluster leader loses all state on restart,
/// then re-initialize it, it should not become the leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn leader_restart_clears_state() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.enable_saving_committed = false;

    let mut log_index;

    // It must initialize a 3-nodes cluster so that it won't become leader in term-1, without election,
    // but instead, it becomes leader in term-2 after a round of election.
    // Otherwise after restart, the restarted leader has greater vote.
    tracing::info!("--- bring up cluster of 3 node");
    {
        router.new_raft_node(0).await;
        router.new_raft_node(1).await;
        router.new_raft_node(2).await;

        tracing::info!("--- initialize node-0 wiht [0,1,2]");
        let n0 = router.get_raft_handle(&0)?;
        n0.initialize(btreemap! {0 => (), 1=>(), 2=>()}).await?;
        log_index = 1; // 0 for initialization log, 1 for leader noop log

        n0.wait(timeout()).state(ServerState::Leader, "node-0 should become leader").await?;
        n0.wait(timeout()).applied_index(Some(log_index), "node-0 applied log").await?;
    }

    tracing::info!(log_index, "--- write to 1 log");
    {
        log_index += router.client_request_many(0, "foo", 1).await?;
    }

    tracing::info!(log_index, "--- stop and restart node-0");
    {
        let (node, _log, _sm): (MemRaft, MemLogStore, MemStateMachine) = router.remove_node(0).unwrap();
        node.shutdown().await?;

        tracing::info!(log_index, "--- restart node-0 with empty log and sm");

        router.new_raft_node(0).await;
        let n0 = router.get_raft_handle(&0)?;

        tracing::info!(log_index, "--- initialize node-0 after restart");
        n0.initialize(btreemap! {0 => (), 1=>(), 2=>()}).await?;

        tracing::info!(log_index, "--- trigger election for node-0 after restart");
        n0.trigger().elect().await?;

        let res = n0.wait(timeout()).state(ServerState::Leader, "should not become leader upon restart").await;
        assert!(res.is_err());
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
