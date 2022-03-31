use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RaftStorageDebug;
use openraft::State;

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
    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());

    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    tracing::info!("--- initializing single node cluster");

    // Wait for node 0 to become leader.
    router.initialize_with(0, btreeset![0]).await?;
    router.wait_for_state(&btreeset![0], State::Leader, None, "init").await?;

    let mut n_logs = 0;
    tracing::info!("--- add one learner");
    router.add_learner(0, 1).await?;
    n_logs += 1;

    tracing::info!("--- write one log");
    router.client_request(0, "foo", 1).await;

    // Wait for metrics to be up to date.
    // Once last_applied updated, the key should be visible in state machine.
    tracing::info!("--- wait for log to sync");
    n_logs += 2u64;
    for node_id in 0..2 {
        router.wait_for_log(&btreeset![node_id], Some(n_logs), None, "write one log").await?;
        let mut sto = router.get_storage_handle(&node_id)?;
        assert!(sto.get_state_machine().await.client_status.get("foo").is_some());
    }

    Ok(())
}
