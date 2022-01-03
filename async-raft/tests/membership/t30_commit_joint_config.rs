use std::sync::Arc;

use anyhow::Result;
use async_raft::Config;
use async_raft::State;
use futures::stream::StreamExt;
use maplit::btreeset;
use tracing_futures::Instrument;

use crate::fixtures::RaftRouter;

/// A leader must wait for learner to commit member-change from [0] to [0,1,2].
/// There is a bug that leader commit a member change log directly because it only checks
/// the replication of voters and believes itself is the only voter.
///
/// - Init 1 leader and 2 learner.
/// - Isolate learner.
/// - Asserts that membership change won't success.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn commit_joint_config_during_0_to_012() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;

    // router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    // Assert all nodes are in learner state & have no entries.
    let want = 1;

    router.wait_for_log(&btreeset![0], want, None, "init node 0").await?;

    // Sync some new nodes.
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    tracing::info!("--- adding new nodes to cluster");
    let mut new_nodes = futures::stream::FuturesUnordered::new();
    new_nodes.push(router.add_learner(0, 1));
    new_nodes.push(router.add_learner(0, 2));
    while let Some(inner) = new_nodes.next().await {
        inner?;
    }

    router.wait_for_log(&btreeset![0], want, None, "init node 0").await?;

    tracing::info!("--- isolate node 1,2, so that membership [0,1,2] wont commit");

    router.isolate_node(1).await;
    router.isolate_node(2).await;

    tracing::info!("--- changing cluster config, should timeout");

    tokio::spawn({
        let router = router.clone();
        async move {
            let _x = router.change_membership(0, btreeset! {0,1,2}).await;
        }
        .instrument(tracing::debug_span!("spawn-change-membership"))
    });

    let res = router
        .wait_for_metrics(
            &0,
            |x| x.last_applied > want,
            None,
            "the next joint log should not commit",
        )
        .await;
    assert!(res.is_err(), "joint log should not commit");

    Ok(())
}

/// Replace membership with another one with only one common node.
/// To reproduce the bug that new config does not actually examine the term/index of learner, but instead only
/// examining the followers
///
/// - bring a cluster of node 0,1,2 online.
/// - isolate 3,4; change config to 2,3,4
/// - since new config can not form a quorum, the joint config should not be committed.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn commit_joint_config_during_012_to_234() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;

    let mut want = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0], want, None, "empty").await?;
    router.wait_for_state(&btreeset![0], State::Learner, None, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    want += 1;

    router.wait_for_log(&btreeset![0], want, None, "init").await?;
    router.assert_stable_cluster(Some(1), Some(want)).await;

    tracing::info!("--- adding 4 new nodes to cluster");

    router.new_raft_node(1).await;
    router.new_raft_node(2).await;
    router.new_raft_node(3).await;
    router.new_raft_node(4).await;

    let mut new_nodes = futures::stream::FuturesUnordered::new();
    new_nodes.push(router.add_learner(0, 1));
    new_nodes.push(router.add_learner(0, 2));
    new_nodes.push(router.add_learner(0, 3));
    new_nodes.push(router.add_learner(0, 4));
    while let Some(inner) = new_nodes.next().await {
        inner?;
    }

    tracing::info!("--- isolate 3,4");

    router.isolate_node(3).await;
    router.isolate_node(4).await;

    tracing::info!("--- changing config to 0,1,2");
    router.change_membership(0, btreeset![0, 1, 2]).await?;
    want += 2;

    router.wait_for_log(&btreeset![0, 1, 2], want, None, "cluster of 0,1,2").await?;

    tracing::info!("--- changing config to 2,3,4");
    {
        let router = router.clone();
        // this is expected to be blocked since 3 and 4 are isolated.
        tokio::spawn(
            async move {
                router.change_membership(0, btreeset![2, 3, 4]).await?;
                Ok::<(), anyhow::Error>(())
            }
            .instrument(tracing::debug_span!("spawn-change-membership")),
        );
    }
    want += 1;

    let wait_rst = router.wait_for_log(&btreeset![0], want, None, "cluster of joint").await;

    // the first step of joint should not pass because the new config can not constitute a quorum
    assert!(wait_rst.is_err());

    Ok(())
}
