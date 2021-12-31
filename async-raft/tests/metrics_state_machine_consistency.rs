use std::sync::Arc;

use anyhow::Result;
use async_raft::Config;
use async_raft::RaftStorageDebug;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::btreeset;

#[macro_use]
mod fixtures;

/// Cluster metrics_state_machine_consistency test.
///
/// What does this test do?
///
/// - brings 2 nodes online: one leader and one non-voter.
/// - write one log to the leader.
/// - asserts that when metrics.last_applied is upto date, the state machine should be upto date too.
///
/// RUST_LOG=async_raft,memstore,metrics_state_machine_consistency=trace cargo test -p async-raft --test
/// metrics_state_machine_consistency
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metrics_state_machine_consistency() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    tracing::info!("--- initializing single node cluster");

    // Wait for node 0 to become leader.
    router.initialize_with(0, btreeset![0]).await?;
    router.wait_for_state(&btreeset![0], State::Leader, None, "init").await?;

    tracing::info!("--- add one non-voter");
    router.add_learner(0, 1).await?;

    tracing::info!("--- write one log");
    router.client_request(0, "foo", 1).await;

    // Wait for metrics to be up to date.
    // Once last_applied updated, the key should be visible in state machine.
    tracing::info!("--- wait for log to sync");
    let want = 2u64;
    for node_id in 0..2 {
        router.wait_for_log(&btreeset![node_id], want, None, "write one log").await?;
        let sto = router.get_storage_handle(&node_id).await?;
        assert!(sto.get_state_machine().await.client_status.get("foo").is_some());
    }

    Ok(())
}
