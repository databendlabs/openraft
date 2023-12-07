use std::option::Option::None;
use std::sync::Arc;

use anyhow::Result;
use futures::stream::StreamExt;
use maplit::btreeset;
use openraft::storage::RaftStateMachine;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::LogId;
use openraft::LogIdOptionExt;
use openraft::Membership;
use openraft::StoredMembership;

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

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    for i in 0..=0 {
        let (_sto, mut sm) = router.get_storage_handle(&i)?;
        assert_eq!(
            StoredMembership::new(
                Some(LogId::new(CommittedLeaderId::new(0, 0), 0)),
                Membership::new(vec![btreeset! {0}], None)
            ),
            sm.applied_state().await?.1
        );
    }

    // Sync some new nodes.
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;
    router.new_raft_node(3).await;
    router.new_raft_node(4).await;

    tracing::info!(log_index, "--- adding new nodes to cluster");
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

    tracing::info!(log_index, "--- changing cluster config");
    let node = router.get_raft_handle(&0)?;
    node.change_membership([0, 1, 2], false).await?;

    log_index += 2;

    tracing::info!(log_index, "--- every node receives joint log");
    for i in 0..5 {
        router
            .wait(&i, None)
            .metrics(|x| x.last_applied.index() >= Some(log_index - 1), "joint log applied")
            .await?;
    }

    tracing::info!(log_index, "--- only 3 node applied membership config");
    for i in 0..3 {
        router
            .wait(&i, None)
            .metrics(|x| x.last_applied.index() == Some(log_index), "uniform log applied")
            .await?;

        let (_sto, mut sm) = router.get_storage_handle(&i)?;
        let (_, last_membership) = sm.applied_state().await?;
        assert_eq!(
            StoredMembership::new(
                Some(LogId::new(CommittedLeaderId::new(1, 0), log_index)),
                Membership::new(vec![btreeset! {0, 1, 2}], Some(btreeset! {3,4}))
            ),
            last_membership
        );
    }

    Ok(())
}
