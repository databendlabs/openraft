use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RaftStorageDebug;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Cluster metrics_state_machine_consistency test.
///
/// What does this test do?
///
/// - brings 2 nodes online: one leader and one learner.
/// - write one log to the leader.
/// - asserts that when metrics.last_applied is upto date, the state machine should be upto date too.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn metrics_state_machine_consistency() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let mut log_index = 0;

    router.new_raft_node(0);
    router.new_raft_node(1);

    tracing::info!("--- initializing single node cluster");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.initialize(btreeset! {0}).await?;
        log_index += 1;

        router.wait(&0, timeout()).state(ServerState::Leader, "n0 -> leader").await?;
    }

    tracing::info!("--- add one learner");
    router.add_learner(0, 1).await?;
    log_index += 1;

    tracing::info!("--- write one log");
    router.client_request(0, "foo", 1).await?;

    // Wait for metrics to be up to date.
    // Once last_applied updated, the key should be visible in state machine.
    tracing::info!("--- wait for log to sync");
    log_index += 1;
    for node_id in 0..2 {
        router.wait_for_log(&btreeset![node_id], Some(log_index), None, "write one log").await?;
        let mut sto = router.get_storage_handle(&node_id)?;
        assert!(sto.get_state_machine().await.client_status.get("foo").is_some());
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
