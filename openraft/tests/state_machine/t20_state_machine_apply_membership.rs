use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::stream::StreamExt;
use maplit::btreeset;
use openraft::Config;
use openraft::EffectiveMembership;
use openraft::LeaderId;
use openraft::LogId;
use openraft::LogIdOptionExt;
use openraft::Membership;
use openraft::RaftStorage;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// All log should be applied to state machine.
///
/// What does this test do?
///
/// - bring a cluster with 3 voter and 2 learner.
/// - check last_membership in state machine.

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn state_machine_apply_membership() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0);

    let mut log_index = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0], None, None, "empty").await?;
    router.wait_for_state(&btreeset![0], ServerState::Learner, None, "empty").await?;
    router.assert_pristine_cluster();

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    log_index += 1;

    router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "init").await?;
    router.assert_stable_cluster(Some(1), Some(log_index));

    for i in 0..=0 {
        let mut sto = router.get_storage_handle(&i)?;
        assert_eq!(
            EffectiveMembership::new(
                Some(LogId::new(LeaderId::new(0, 0), 0)),
                Membership::new(vec![btreeset! {0}], None)
            ),
            sto.last_applied_state().await?.1
        );
    }

    // Sync some new nodes.
    router.new_raft_node(1);
    router.new_raft_node(2);
    router.new_raft_node(3);
    router.new_raft_node(4);

    tracing::info!("--- adding new nodes to cluster");
    let mut new_nodes = futures::stream::FuturesUnordered::new();
    new_nodes.push(router.add_learner(0, 1));
    new_nodes.push(router.add_learner(0, 2));
    new_nodes.push(router.add_learner(0, 3));
    new_nodes.push(router.add_learner(0, 4));
    while let Some(inner) = new_nodes.next().await {
        inner?;
    }
    log_index += 4;
    router.wait_for_log(&btreeset![0], Some(log_index), None, "add learner").await?;

    tracing::info!("--- changing cluster config");
    let node = router.get_raft_handle(&0)?;
    node.change_membership(btreeset![0, 1, 2], true, false).await?;

    log_index += 2;

    tracing::info!("--- every node receives joint log");
    for i in 0..5 {
        router
            .wait(&i, None)
            .metrics(|x| x.last_applied.index() >= Some(log_index - 1), "joint log applied")
            .await?;
    }

    tracing::info!("--- only 3 node applied membership config");
    for i in 0..3 {
        router
            .wait(&i, None)
            .metrics(|x| x.last_applied.index() == Some(log_index), "uniform log applied")
            .await?;

        let mut sto = router.get_storage_handle(&i)?;
        let (_, last_membership) = sto.last_applied_state().await?;
        assert_eq!(
            EffectiveMembership::new(
                Some(LogId::new(LeaderId::new(1, 0), log_index)),
                Membership::new(vec![btreeset! {0, 1, 2}], Some(btreeset! {3,4}))
            ),
            last_membership
        );
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
