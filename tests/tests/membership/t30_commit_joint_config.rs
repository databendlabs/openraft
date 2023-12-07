use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::stream::StreamExt;
use maplit::btreeset;
use openraft::Config;
use openraft::LogIdOptionExt;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// A leader must wait for learner to commit member-change from [0] to [0,1,2].
/// There is a bug that leader commit a member change log directly because it only checks
/// the replication of voters and believes itself is the only voter.
///
/// - Init 1 leader and 2 learner.
/// - Isolate learner.
/// - Asserts that membership change won't success.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn commit_joint_config_during_0_to_012() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize(0).await?;
    // Assert all nodes are in learner state & have no entries.
    let mut log_index = 1;

    router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "init node 0").await?;

    // Sync some new nodes.
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    tracing::info!(log_index, "--- adding new nodes to cluster");
    let mut new_nodes = futures::stream::FuturesUnordered::new();
    new_nodes.push(router.add_learner(0, 1));
    new_nodes.push(router.add_learner(0, 2));
    while let Some(inner) = new_nodes.next().await {
        inner?;
    }
    log_index += 2;

    router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "init node 0").await?;

    tracing::info!(
        log_index,
        "--- isolate node 1,2, so that membership [0,1,2] wont commit"
    );

    router.set_network_error(1, true);
    router.set_network_error(2, true);

    tracing::info!(log_index, "--- changing cluster config, should timeout");

    tokio::spawn({
        let router = router.clone();
        async move {
            let node = router.get_raft_handle(&0).unwrap();
            let _x = node.change_membership([0, 1, 2], false).await;
        }
        .instrument(tracing::debug_span!("spawn-change-membership"))
    });

    let res = router
        .wait_for_metrics(
            &0,
            |x| x.last_applied.index() > Some(log_index),
            timeout(),
            "the next joint log should not commit",
        )
        .await;
    assert!(res.is_err(), "joint log should not commit");

    Ok(())
}

/// Replace membership with another one with only one common node.
/// To reproduce the bug that new config does not actually examine the term/index of learner, but
/// instead only examining the followers
///
/// - bring a cluster of node 0,1,2 online.
/// - isolate 3,4; change config to 2,3,4
/// - since new config can not form a quorum, the joint config should not be committed.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn commit_joint_config_during_012_to_234() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let mut log_index = router.new_cluster(btreeset! {0,1,2,3,4}, btreeset! {}).await?;

    tracing::info!(log_index, "--- isolate 3,4");

    router.set_network_error(3, true);
    router.set_network_error(4, true);

    tracing::info!(log_index, "--- changing config to 0,1,2");
    let node = router.get_raft_handle(&0)?;
    node.change_membership([0, 1, 2], false).await?;
    log_index += 2;

    router.wait_for_log(&btreeset![0, 1, 2], Some(log_index), None, "cluster of 0,1,2").await?;

    tracing::info!(log_index, "--- changing config to 2,3,4");
    {
        let router = router.clone();
        // this is expected to be blocked since 3 and 4 are isolated.
        tokio::spawn(
            async move {
                let node = router.get_raft_handle(&0)?;
                node.change_membership([2, 3, 4], false).await?;
                Ok::<(), anyhow::Error>(())
            }
            .instrument(tracing::debug_span!("spawn-change-membership")),
        );
    }
    log_index += 2;

    let wait_rst = router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "cluster of joint").await;

    // the first step of joint should not pass because the new config can not constitute a quorum
    assert!(wait_rst.is_err());

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
