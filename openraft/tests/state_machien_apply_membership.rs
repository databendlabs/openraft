use std::sync::Arc;

use anyhow::Result;
use fixtures::RaftRouter;
use futures::stream::StreamExt;
use maplit::btreeset;
use openraft::raft::Membership;
use openraft::Config;
use openraft::EffectiveMembership;
use openraft::LogId;
use openraft::RaftStorage;
use openraft::State;

#[macro_use]
mod fixtures;

/// All log should be applied to state machine.
///
/// What does this test do?
///
/// - bring a cluster with 3 voter and 2 learner.
/// - check last_membership in state machine.

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn state_machine_apply_membership() -> Result<()> {
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

    for i in 0..=0 {
        let sto = router.get_storage_handle(&i).await?;
        assert_eq!(
            Some(EffectiveMembership {
                log_id: LogId { term: 1, index: 1 },
                membership: Membership::new_single(btreeset! {0})
            }),
            sto.last_applied_state().await?.1
        );
    }

    // Sync some new nodes.
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;
    router.new_raft_node(3).await;
    router.new_raft_node(4).await;

    tracing::info!("--- adding new nodes to cluster");
    let mut new_nodes = futures::stream::FuturesUnordered::new();
    new_nodes.push(router.add_learner(0, 1));
    new_nodes.push(router.add_learner(0, 2));
    new_nodes.push(router.add_learner(0, 3));
    new_nodes.push(router.add_learner(0, 4));
    while let Some(inner) = new_nodes.next().await {
        inner?;
    }

    tracing::info!("--- changing cluster config");
    router.change_membership(0, btreeset![0, 1, 2]).await?;
    want += 2;

    // router.wait_for_log(&btreeset![0, 1, 2, 3, 4], want, None, "cluster of 5 candidates").await?;

    tracing::info!("--- every node receives joint log");
    for i in 0..5 {
        router.wait(&i, None).await?.metrics(|x| x.last_applied >= want - 1, "joint log applied").await?;
    }

    tracing::info!("--- only 3 node applied membership config");
    for i in 0..3 {
        router.wait(&i, None).await?.metrics(|x| x.last_applied == want, "uniform log applied").await?;

        let sto = router.get_storage_handle(&i).await?;
        let (_, last_membership) = sto.last_applied_state().await?;
        assert_eq!(
            Some(EffectiveMembership {
                log_id: LogId { term: 1, index: 3 },
                membership: Membership::new_single(btreeset! {0,1,2})
            }),
            last_membership
        );
    }

    Ok(())
}
