use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Lagging network test.
///
/// What does this test do?
///
/// - Setup a network with <=50 ms random delay of messages.
/// - bring a single-node cluster online.
/// - add two Learner and then try to commit one log.
/// - change config to a 3 members cluster and commit another log.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn lagging_network_write() -> Result<()> {
    let config = Arc::new(
        Config {
            heartbeat_interval: 100,
            election_timeout_min: 300,
            election_timeout_max: 600,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::builder(config).send_delay(50).build();

    router.new_raft_node(0);
    let mut log_index = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0], None, timeout(), "empty").await?;
    router.wait_for_state(&btreeset![0], ServerState::Learner, None, "empty").await?;
    router.assert_pristine_cluster();

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    log_index += 1; // log 0: initial membership log; log 1: leader commits a blank log

    router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "init").await?;
    router.wait_for_state(&btreeset![0], ServerState::Leader, None, "init").await?;
    router.assert_stable_cluster(Some(1), Some(log_index));

    // Sync some new nodes.
    router.new_raft_node(1);
    router.add_learner(0, 1).await?;
    log_index += 1;

    router.new_raft_node(2);
    router.add_learner(0, 2).await?;
    log_index += 1;

    router.wait_for_log(&btreeset![1, 2], Some(log_index), timeout(), "learner init").await?;

    router.client_request_many(0, "client", 1).await?;
    log_index += 1;
    router.wait_for_log(&btreeset![0, 1, 2], Some(log_index), timeout(), "write one log").await?;

    let node = router.get_raft_handle(&0)?;
    node.change_membership(btreeset![0, 1, 2], true, false).await?;
    log_index += 2;
    router.wait_for_state(&btreeset![0], ServerState::Leader, None, "changed").await?;
    router.wait_for_state(&btreeset![1, 2], ServerState::Follower, None, "changed").await?;
    router.wait_for_log(&btreeset![0, 1, 2], Some(log_index), timeout(), "3 candidates").await?;

    router.client_request_many(0, "client", 1).await?;
    log_index += 1;
    router.wait_for_log(&btreeset![0, 1, 2], Some(log_index), timeout(), "write 2nd log").await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
