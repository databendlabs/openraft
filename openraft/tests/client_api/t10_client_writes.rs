use std::sync::Arc;

use anyhow::Result;
use futures::prelude::*;
use maplit::btreeset;
use openraft::Config;
use openraft::LogId;
use openraft::SnapshotPolicy;
use openraft::State;

use crate::fixtures::RaftRouter;

/// Client write tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - write a lot of data to it.
/// - assert that the cluster stayed stable and has all of the expected data.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_writes() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

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
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let mut n_logs = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1, 2], None, None, "empty").await?;
    router.wait_for_state(&btreeset![0, 1, 2], State::Learner, None, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    n_logs += 1;

    router.wait_for_log(&btreeset![0, 1, 2], Some(n_logs), None, "leader init log").await?;
    router.wait_for_state(&btreeset![0], State::Leader, None, "cluster leader").await?;
    router.wait_for_state(&btreeset![1, 2], State::Follower, None, "cluster follower").await?;

    router.assert_stable_cluster(Some(1), Some(n_logs)).await;

    // Write a bunch of data and assert that the cluster stayes stable.
    let leader = router.leader().await.expect("leader not found");
    let mut clients = futures::stream::FuturesUnordered::new();
    clients.push(router.client_request_many(leader, "0", 500));
    clients.push(router.client_request_many(leader, "1", 500));
    clients.push(router.client_request_many(leader, "2", 500));
    clients.push(router.client_request_many(leader, "3", 500));
    clients.push(router.client_request_many(leader, "4", 500));
    clients.push(router.client_request_many(leader, "5", 500));
    while clients.next().await.is_some() {}

    n_logs += 500 * 6;
    router.wait_for_log(&btreeset![0, 1, 2], Some(n_logs), None, "sync logs").await?;

    router.assert_stable_cluster(Some(1), Some(n_logs)).await; // The extra 1 is from the leader's initial commit entry.

    router
        .assert_storage_state(
            1,
            n_logs,
            Some(0),
            LogId::new(1, n_logs),
            Some(((1999..2100).into(), 1)),
        )
        .await?;

    Ok(())
}
