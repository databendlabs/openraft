use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;

use crate::fixtures::MemLogStore;
use crate::fixtures::MemRaft;
use crate::fixtures::MemStateMachine;
use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Test follower restart after state cleared to be able to recovery.
///
/// A 3-nodes cluster follower loses all state on restart,
/// then the leader should be able to discover its state loss via heartbeat and recover it.
///
/// This is meant to address the issue a `conflict` response for a heartbeat does not reset the
/// transmission progress.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn follower_clear_restart_recover() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            allow_log_reversion: Some(true),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.enable_saving_committed = false;

    tracing::info!("--- bring up cluster of 3 node");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- stop node-1");
    {
        let (n1, _log, _sm): (MemRaft, MemLogStore, MemStateMachine) = router.remove_node(1).unwrap();
        n1.shutdown().await?;
    }

    tracing::info!(log_index, "--- restart node-1 with empty log and sm");
    router.new_raft_node(1).await;

    tracing::info!(
        log_index,
        "--- trigger heartbeat on node-0 to let it discover the state loss on node-1"
    );
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().heartbeat().await?;

        let n1 = router.get_raft_handle(&1)?;
        n1.wait(timeout()).log_index(Some(log_index), "should recovered").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
