use std::sync::Arc;

use anyhow::Result;
use maplit::hashset;

use async_raft::{Config, State};
use fixtures::RaftRouter;

mod fixtures;

/// Cluster metrics_state_machine_consistency test.
///
/// What does this test do?
///
/// - brings 2 nodes online: one leader and one non-voter.
/// - write one log to the leader.
/// - asserts that when metrics.last_applied is upto date, the state machine should be upto date too.
///
/// RUST_LOG=async_raft,memstore,metrics_state_machine_consistency=trace cargo test -p async-raft --test metrics_state_machine_consistency
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metrics_state_machine_consistency() -> Result<()> {
    fixtures::init_tracing();

    // Setup test dependencies.
    let config = Arc::new(Config::build("test".into()).validate().expect("failed to build Raft config"));
    let router = Arc::new(RaftRouter::new(config.clone()));

    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    tracing::info!("--- initializing single node cluster");

    // Wait for node 0 to become leader.
    router.initialize_with(0, hashset![0]).await?;
    router.wait_for_state(&hashset![0], State::Leader, "init").await?;

    tracing::info!("--- add one non-voter");
    router.add_non_voter(0, 1).await?;

    tracing::info!("--- write one log");
    router.client_request(0, "foo", 1).await;

    // Wait for metrics to be up to date.
    // Once last_applied updated, the key should be visible in state machine.
    tracing::info!("--- wait for log to sync");
    let want = 2u64;
    for node_id in 0..2 {
        router.wait_for_log(&hashset![node_id], want, "write one log").await?;
        let sto = router.get_storage_handle(&node_id).await?;
        assert!(sto.get_state_machine().await.client_status.get("foo").is_some());
    }

    Ok(())
}
