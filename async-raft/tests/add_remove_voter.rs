use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_raft::Config;
use async_raft::State;
use fixtures::RaftRouter;
use futures::stream::StreamExt;
use maplit::btreeset;

#[macro_use]
mod fixtures;

/// Cluster add_remove_voter test.
///
/// What does this test do?
///
/// - brings 5 nodes online: one leader and 4 non-voter.
/// - add 4 non-voter as follower.
/// - asserts that the leader was able to successfully commit logs and that the followers has successfully replicated
///   the payload.
/// - remove one follower: node-4
/// - asserts node-4 becomes non-voter and the leader stops sending logs to it.
///
/// RUST_LOG=async_raft,memstore,add_remove_voter=trace cargo test -p async-raft --test add_remove_voter
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn add_remove_voter() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let timeout = Duration::from_millis(500);
    let all_members = btreeset![0, 1, 2, 3, 4];
    let left_members = btreeset![0, 1, 2, 3];

    // Setup test dependencies.
    let config = Arc::new(
        Config::build("test".into())
            // .election_timeout_max(50)
            // .election_timeout_min(20)
            // .heartbeat_interval(10)
            .validate()
            .expect("failed to build Raft config"),
    );
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;

    // Assert all nodes are in non-voter state & have no entries.
    let mut want = 0;
    router
        .wait_for_metrics(
            &0u64,
            |x| x.last_log_index == want,
            Some(timeout),
            &format!("n{}.last_log_index -> {}", 0, 0),
        )
        .await?;
    router
        .wait_for_metrics(
            &0u64,
            |x| x.state == State::NonVoter,
            Some(timeout),
            &format!("n{}.state -> {:?}", 4, State::NonVoter),
        )
        .await?;

    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    want = 1;

    wait_log(router.clone(), &btreeset![0], want).await?;
    router.assert_stable_cluster(Some(1), Some(want)).await;

    // Sync some new nodes.
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;
    router.new_raft_node(3).await;
    router.new_raft_node(4).await;

    tracing::info!("--- adding new nodes to cluster");
    let mut new_nodes = futures::stream::FuturesUnordered::new();
    new_nodes.push(router.add_non_voter(0, 1));
    new_nodes.push(router.add_non_voter(0, 2));
    new_nodes.push(router.add_non_voter(0, 3));
    new_nodes.push(router.add_non_voter(0, 4));
    while let Some(inner) = new_nodes.next().await {
        inner?;
    }

    wait_log(router.clone(), &all_members, want).await?;

    tracing::info!("--- changing cluster config");
    router.change_membership(0, all_members.clone()).await?;
    want += 2; // 2 member-change logs

    wait_log(router.clone(), &all_members, want).await?;
    router.assert_stable_cluster(Some(1), Some(want)).await; // Still in term 1, so leader is still node 0.

    // Send some requests
    router.client_request_many(0, "client", 100).await;
    want += 100;

    wait_log(router.clone(), &all_members, want).await?;

    // Remove Node 4
    tracing::info!("--- remove n{}", 4);
    router.change_membership(0, left_members.clone()).await?;
    want += 2; // two member-change logs

    wait_log(router.clone(), &left_members, want).await?;
    router
        .wait_for_metrics(
            &4u64,
            |x| x.state == State::NonVoter,
            Some(timeout),
            &format!("n{}.state -> {:?}", 4, State::NonVoter),
        )
        .await?;

    // Send some requests
    router.client_request_many(0, "client", 100).await;
    want += 100;

    wait_log(router.clone(), &left_members, want).await?;

    // log will not be sync to removed node
    let x = router.latest_metrics().await;
    assert!(x[4].last_log_index < want);
    Ok(())
}

async fn wait_log(router: std::sync::Arc<fixtures::RaftRouter>, node_ids: &BTreeSet<u64>, want_log: u64) -> Result<()> {
    let timeout = Duration::from_millis(500);
    for i in node_ids.iter() {
        router
            .wait_for_metrics(
                &i,
                |x| x.last_log_index == want_log,
                Some(timeout),
                &format!("n{}.last_log_index -> {}", i, want_log),
            )
            .await?;
        router
            .wait_for_metrics(
                &i,
                |x| x.last_applied == want_log,
                Some(timeout),
                &format!("n{}.last_applied -> {}", i, want_log),
            )
            .await?;
    }
    Ok(())
}
