use std::sync::Arc;

use anyhow::Result;
use futures::prelude::*;
use maplit::btreeset;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::SnapshotPolicy;
use openraft::State;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Client write tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - write a lot of data to it.
/// - assert that the cluster stayed stable and has all of the expected data.
#[async_entry::test(worker_threads = 4, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn client_writes() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(2000),
            // The write load is heavy in this test, need a relatively long timeout.
            election_timeout_min: 500,
            election_timeout_max: 1000,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let mut log_index = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1, 2], None, None, "empty").await?;
    router.wait_for_state(&btreeset![0, 1, 2], State::Learner, None, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    log_index += 1;

    router.wait_for_log(&btreeset![0, 1, 2], Some(log_index), None, "leader init log").await?;
    router.wait_for_state(&btreeset![0], State::Leader, None, "cluster leader").await?;
    router.wait_for_state(&btreeset![1, 2], State::Follower, None, "cluster follower").await?;

    router.assert_stable_cluster(Some(1), Some(log_index)).await;

    // Write a bunch of data and assert that the cluster stayes stable.
    let leader = router.leader().expect("leader not found");
    let mut clients = futures::stream::FuturesUnordered::new();
    clients.push(router.client_request_many(leader, "0", 500));
    clients.push(router.client_request_many(leader, "1", 500));
    clients.push(router.client_request_many(leader, "2", 500));
    clients.push(router.client_request_many(leader, "3", 500));
    clients.push(router.client_request_many(leader, "4", 500));
    clients.push(router.client_request_many(leader, "5", 500));
    while clients.next().await.is_some() {}

    log_index += 500 * 6;
    router.wait_for_log(&btreeset![0, 1, 2], Some(log_index), None, "sync logs").await?;

    router.assert_stable_cluster(Some(1), Some(log_index)).await; // The extra 1 is from the leader's initial commit entry.

    router
        .assert_storage_state(
            1,
            log_index,
            Some(0),
            LogId::new(LeaderId::new(1, 0), log_index),
            Some(((1999..2100).into(), 1)),
        )
        .await?;

    Ok(())
}
