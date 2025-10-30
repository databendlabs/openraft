use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Lagging network test.
///
/// What does this test do?
///
/// - Setup a network with <=50 ms random delay of messages.
/// - bring a single-node cluster online.
/// - add two Learner and then try to commit one log.
/// - change config to a 3 members cluster and commit another log.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
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

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {1,2}).await?;

    router.client_request_many(0, "client", 1).await?;
    log_index += 1;
    for id in [0, 1, 2] {
        router.wait(&id, timeout()).applied_index(Some(log_index), "write one log").await?;
    }

    let node = router.get_raft_handle(&0)?;
    node.change_membership([0, 1, 2], false).await?;
    log_index += 2;
    router.wait(&0, None).state(ServerState::Leader, "changed").await?;
    for node in [1, 2] {
        router.wait(&node, None).state(ServerState::Follower, "changed").await?;
    }
    for id in [0, 1, 2] {
        router.wait(&id, timeout()).applied_index(Some(log_index), "3 candidates").await?;
    }

    router.client_request_many(0, "client", 1).await?;
    log_index += 1;
    for id in [0, 1, 2] {
        router.wait(&id, timeout()).applied_index(Some(log_index), "write 2nd log").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
