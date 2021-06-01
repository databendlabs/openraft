use std::sync::Arc;

use anyhow::Result;
use async_raft::raft::MembershipConfig;
use async_raft::Config;
use async_raft::State;
use fixtures::RaftRouter;
use futures::prelude::*;
use maplit::hashset;

mod fixtures;

/// Client write tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - write a lot of data to it.
/// - assert that the cluster stayed stable and has all of the expected data.
///
/// RUST_LOG=async_raft,memstore,client_writes=trace cargo test -p async-raft --test client_writes
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_writes() -> Result<()> {
    fixtures::init_tracing();

    // Setup test dependencies.
    let config = Arc::new(
        Config::build("test".into())
            .validate()
            .expect("failed to build Raft config"),
    );
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let mut want = 0;

    // Assert all nodes are in non-voter state & have no entries.
    router
        .wait_for_log(&hashset![0, 1, 2], want, "empty")
        .await?;
    router
        .wait_for_state(&hashset![0, 1, 2], State::NonVoter, "empty")
        .await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    want += 1;

    router
        .wait_for_log(&hashset![0, 1, 2], want, "leader init log")
        .await?;
    router
        .wait_for_state(&hashset![0], State::Leader, "init")
        .await?;

    router.assert_stable_cluster(Some(1), Some(want)).await;

    // Write a bunch of data and assert that the cluster stayes stable.
    let leader = router.leader().await.expect("leader not found");
    let mut clients = futures::stream::FuturesUnordered::new();
    clients.push(router.client_request_many(leader, "0", 1000));
    clients.push(router.client_request_many(leader, "1", 1000));
    clients.push(router.client_request_many(leader, "2", 1000));
    clients.push(router.client_request_many(leader, "3", 1000));
    clients.push(router.client_request_many(leader, "4", 1000));
    clients.push(router.client_request_many(leader, "5", 1000));
    while clients.next().await.is_some() {}

    want = 6001;
    router
        .wait_for_log(&hashset![0, 1, 2], want, "sync logs")
        .await?;

    router.assert_stable_cluster(Some(1), Some(want)).await; // The extra 1 is from the leader's initial commit entry.
    router
        .assert_storage_state(
            1,
            want,
            Some(0),
            want,
            Some(((5000..5100).into(), 1, MembershipConfig {
                members: hashset![0, 1, 2],
                members_after_consensus: None,
            })),
        )
        .await;

    Ok(())
}
