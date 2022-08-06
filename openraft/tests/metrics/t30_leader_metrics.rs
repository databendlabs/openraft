use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::stream::StreamExt;
use maplit::btreemap;
use maplit::btreeset;
use openraft::metrics::ReplicationTargetMetrics;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::ServerState;
#[allow(unused_imports)] use pretty_assertions::assert_eq;
#[allow(unused_imports)] use pretty_assertions::assert_ne;

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
    let c01234 = btreeset![0, 1, 2, 3, 4];
    let c0123 = btreeset![0, 1, 2, 3];

    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0);

    // Assert all nodes are in learner state & have no entries.
    let mut log_index = 0;

    router.wait_for_log(&btreeset![0], None, timeout(), "init").await?;
    router.wait_for_state(&btreeset![0], ServerState::Learner, timeout(), "init").await?;

    router.assert_pristine_cluster();

    tracing::info!("--- initializing cluster");

    router.initialize_from_single_node(0).await?;
    log_index += 1;

    router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "init cluster").await?;
    router.assert_stable_cluster(Some(1), Some(log_index));

    router
        .wait_for_metrics(
            &0,
            |x| {
                if let Some(ref q) = x.replication {
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
    router.new_raft_node(1);
    router.new_raft_node(2);
    router.new_raft_node(3);
    router.new_raft_node(4);

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
    router.wait_for_log(&c01234, Some(log_index), timeout(), "add learner 1,2,3,4").await?;

    tracing::info!("--- changing cluster config to 01234");

    let node = router.get_raft_handle(&0)?;
    node.change_membership(c01234.clone(), true, false).await?;
    log_index += 2; // 2 member-change logs

    router.wait_for_log(&c01234, Some(log_index), timeout(), "change members to 0,1,2,3,4").await?;

    router.assert_stable_cluster(Some(1), Some(log_index)); // Still in term 1, so leader is still node 0.

    let ww = ReplicationTargetMetrics::new(LogId::new(LeaderId::new(1, 0), log_index));
    let want_repl = btreemap! { 1=>ww.clone(), 2=>ww.clone(), 3=>ww.clone(), 4=>ww.clone(), };
    router
        .wait_for_metrics(
            &0,
            |x| {
                if let Some(ref q) = x.replication {
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
    router.client_request_many(0, "client", 10).await?;
    log_index += 10;

    tracing::info!("--- remove n{}", 4);
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership(c0123.clone(), true, false).await?;
        log_index += 2; // two member-change logs

        router
            .wait_for_log(
                &c0123,
                Some(log_index),
                timeout(),
                "other nodes should commit the membership change log",
            )
            .await?;
    }

    tracing::info!("--- replication metrics should reflect the replication state");
    {
        let ww = ReplicationTargetMetrics::new(LogId::new(LeaderId::new(1, 0), log_index));
        let want_repl = btreemap! { 1=>ww.clone(), 2=>ww.clone(), 3=>ww.clone()};
        router
            .wait_for_metrics(
                &0,
                |x| {
                    if let Some(ref q) = x.replication {
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

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;

    tracing::info!("--- let node-1 to elect to take leadership from node-0");
    {
        n1.trigger_elect().await?;
        n1.wait(timeout()).state(ServerState::Leader, "node-1 becomes leader").await?;
        n1.wait(timeout()).metrics(|x| x.replication.is_some(), "node-1 starts replication").await?;

        n0.wait(timeout()).metrics(|x| x.replication.is_none(), "node-0 stopped replication").await?;
        n0.wait(timeout())
            .metrics(|x| x.current_leader == Some(1), "node-0 receives leader-1 message")
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
