use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::stream::StreamExt;
use maplit::btreemap;
use maplit::btreeset;
use openraft::raft::VoteRequest;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use openraft::ReplicationMetrics;
use openraft::State;
use openraft::Vote;
#[allow(unused_imports)]
use pretty_assertions::assert_eq;
#[allow(unused_imports)]
use pretty_assertions::assert_ne;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Cluster leader_metrics test.
///
/// What does this test do?
///
/// - brings 5 nodes online: one leader and 4 learner.
/// - add 4 learner as follower.
/// - asserts that the leader was able to successfully commit logs and that the followers has successfully replicated
///   the payload.
/// - remove one follower: node-4
/// - asserts node-4 becomes learner and the leader stops sending logs to it.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn leader_metrics() -> Result<()> {
    let span = tracing::debug_span!("leader_metrics");
    let _ent = span.enter();

    let all_members = btreeset![0, 1, 2, 3, 4];
    let left_members = btreeset![0, 1, 2, 3];

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    // Assert all nodes are in learner state & have no entries.
    let mut log_index = 0;

    router.wait_for_log(&btreeset![0], None, timeout(), "init").await?;
    router.wait_for_state(&btreeset![0], State::Learner, timeout(), "init").await?;

    router.assert_pristine_cluster().await;

    tracing::info!("--- initializing cluster");

    router.initialize_from_single_node(0).await?;
    log_index += 1;

    router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "init cluster").await?;
    router.assert_stable_cluster(Some(1), Some(log_index)).await;

    router
        .wait_for_metrics(
            &0,
            |x| {
                if let Some(ref q) = x.leader_metrics {
                    q.data().replication.is_empty()
                } else {
                    false
                }
            },
            timeout(),
            "no replication with 1 node cluster",
        )
        .await?;

    // Sync some new nodes.
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;
    router.new_raft_node(3).await;
    router.new_raft_node(4).await;

    tracing::info!("--- adding 4 new nodes to cluster");

    {
        let mut new_nodes = futures::stream::FuturesUnordered::new();
        new_nodes.push(router.add_learner(0, 1));
        new_nodes.push(router.add_learner(0, 2));
        new_nodes.push(router.add_learner(0, 3));
        new_nodes.push(router.add_learner(0, 4));
        while let Some(inner) = new_nodes.next().await {
            inner?;
        }
    }
    log_index += 4; // 4 add_learner log
    router.wait_for_log(&all_members, Some(log_index), timeout(), "add learner 1,2,3,4").await?;

    tracing::info!("--- changing cluster config to 012");

    let node = router.get_raft_handle(&0)?;
    node.change_membership(all_members.clone(), true, false).await?;
    log_index += 2; // 2 member-change logs

    router.wait_for_log(&all_members, Some(log_index), timeout(), "change members to 0,1,2,3,4").await?;

    router.assert_stable_cluster(Some(1), Some(log_index)).await; // Still in term 1, so leader is still node 0.

    let ww = ReplicationMetrics::new(LogId::new(LeaderId::new(1, 0), log_index));
    let want_repl = btreemap! { 1=>ww.clone(), 2=>ww.clone(), 3=>ww.clone(), 4=>ww.clone(), };
    router
        .wait_for_metrics(
            &0,
            |x| {
                if let Some(ref q) = x.leader_metrics {
                    q.data().replication == want_repl
                } else {
                    false
                }
            },
            timeout(),
            "replication metrics to 4 nodes",
        )
        .await?;

    // Send some requests
    router.client_request_many(0, "client", 10).await;
    log_index += 10;

    tracing::info!("--- remove n{}", 4);
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership(left_members.clone(), true, false).await?;
        log_index += 2; // two member-change logs

        tracing::info!("--- n{} should revert to learner", 4);
        router
            .wait_for_metrics(
                &4,
                |x| x.state == State::Learner,
                timeout(),
                &format!("n{}.state -> {:?}", 4, State::Learner),
            )
            .await?;

        router
            .wait_for_log(
                &left_members,
                Some(log_index),
                timeout(),
                "other nodes should commit the membership change log",
            )
            .await?;
    }

    tracing::info!("--- replication metrics should reflect the replication state");
    {
        let ww = ReplicationMetrics::new(LogId::new(LeaderId::new(1, 0), log_index));
        let want_repl = btreemap! { 1=>ww.clone(), 2=>ww.clone(), 3=>ww.clone()};
        router
            .wait_for_metrics(
                &0,
                |x| {
                    if let Some(ref q) = x.leader_metrics {
                        q.data().replication == want_repl
                    } else {
                        false
                    }
                },
                timeout(),
                "replication metrics to 3 nodes",
            )
            .await?;
    }

    let leader = router.current_leader(0).await.unwrap();

    tracing::info!("--- take leadership of node {}", leader);
    {
        router
            .connect(leader, None)
            .await
            .send_vote(VoteRequest {
                vote: Vote::new(100, 100),
                last_log_id: Some(LogId::new(LeaderId::new(10, 0), 100)),
            })
            .await?;

        router
            .wait_for_metrics(
                &leader,
                |x| x.leader_metrics.is_none(),
                timeout(),
                "node 0 should close all replication",
            )
            .await?;

        // The next election may have finished before waiting.
        router
            .wait_for_metrics(
                &leader,
                |x| x.state != State::Leader || (x.state == State::Leader && x.current_term > 100),
                timeout(),
                &format!("node {} becomes candidate or becomes a new leader", leader,),
            )
            .await?;

        router
            .wait(&leader, timeout())
            .await?
            .metrics(|x| x.current_leader.is_some(), "elect new leader")
            .await?;
    }

    tracing::info!("--- check leader metrics after leadership transferred.");
    let leader = router.current_leader(0).await.unwrap();
    tracing::info!("--- new leader is {}", leader);

    router
        .wait_for_metrics(
            &leader,
            |x| x.leader_metrics.is_some(),
            timeout(),
            "new leader spawns replication",
        )
        .await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
