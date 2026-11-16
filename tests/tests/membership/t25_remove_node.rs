use std::collections::BTreeMap;

use anyhow::Result;
use openraft::Config;
use openraft::raft::RemovedVoterPolicy;
use openraft::storage::RaftStateMachine;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Permanent removal covers a voter, a retained learner, retries, and restart.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn removal_covers_voters_and_retained_learners() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0, 1, 2], [3]).await?;
    let leader = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- demote voter 2 while retaining its replication");
    {
        let response = leader.change_membership_with_policy([0, 1], RemovedVoterPolicy::KeepAsLearner).await?;
        let membership = response.membership.unwrap();
        let ids: BTreeMap<_, _> = membership.nodes().map(|(id, node)| (*id, *node)).collect();
        let expected = BTreeMap::from([(0, ()), (1, ()), (2, ()), (3, ())]);
        assert_eq!(ids, expected);
    }

    tracing::info!("--- remove retained learner 2, voter 1, and existing learner 3");
    {
        leader.remove_node(2).await?;
        leader.remove_node(1).await?;
        leader.remove_node(3).await?;
        let response = leader.remove_node(2).await?;
        router.wait(&0).applied_index(Some(response.log_id.index()), "all removals applied").await?;
        let (_, mut state) = router.get_storage_handle(&0)?;
        let (_, membership) = state.applied_state().await?;
        let actual: BTreeMap<_, _> = membership.membership().nodes().map(|(id, node)| (*id, *node)).collect();
        let expected = BTreeMap::from([(0, ())]);
        assert_eq!(actual, expected);
    }

    tracing::info!("--- restart the remaining node with its applied membership");
    {
        let node = router.stop_node(0).await?;
        router.new_raft_node_with_sto(0, node.log_store, node.state_machine).await?;
        let (_, mut state) = router.get_storage_handle(&0)?;
        let (_, membership) = state.applied_state().await?;
        let actual: BTreeMap<_, _> = membership.membership().nodes().map(|(id, node)| (*id, *node)).collect();
        let expected = BTreeMap::from([(0, ())]);
        assert_eq!(actual, expected);
    }
    Ok(())
}

/// A named permanent-removal policy removes former voters while preserving learners.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn permanent_policy_preserves_existing_learners() -> Result<()> {
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0, 1, 2], [3]).await?;
    let leader = router.get_raft_handle(&0)?;
    tracing::info!(
        log_index,
        "--- permanently remove voter 2 and retain existing learner 3"
    );
    let response = leader.change_membership_with_policy([0, 1], RemovedVoterPolicy::Remove).await?;
    let membership = response.membership.unwrap();
    let actual: BTreeMap<_, _> = membership.nodes().map(|(id, node)| (*id, *node)).collect();
    let expected = BTreeMap::from([(0, ()), (1, ()), (3, ())]);
    assert_eq!(actual, expected);
    Ok(())
}
