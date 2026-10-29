use anyhow::Result;
use maplit::btreemap;
use maplit::btreeset;
use openraft::alias::AsyncRuntimeOf;
use openraft::membership::NodeRole;
use openraft_membership::GridChange;
use openraft_membership::GridMembership;

use crate::fixtures::custom_membership::ChangeScenario;
use crate::fixtures::custom_membership::QuorumScenario;
use crate::fixtures::ut_harness;

openraft::declare_raft_types!(
    StdConfig:
        D = String,
        R = (),
        Node = (),
        LeaderId = openraft::impls::leader_id_std::LeaderId<u64, u64>,
        Membership = GridMembership<u64, ()>,
        MembershipEdit = GridChange<Self::NodeId, Self::Node>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

openraft::declare_raft_types!(
    AdvConfig:
        D = String,
        R = (),
        Node = (),
        Membership = GridMembership<u64, ()>,
        MembershipEdit = GridChange<Self::NodeId, Self::Node>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

/// Voters 1 to 4 in the grid `[[1,2],[3,4]]`. A read quorum holds a row, and a write quorum a
/// voter of each row, such as `{1,4}` from two columns.
fn square() -> Result<QuorumScenario<GridMembership<u64, ()>>> {
    let node_ids = btreeset! {1, 2, 3, 4};
    let membership = GridMembership::new(vec![vec![1, 2], vec![3, 4]], node_ids.clone())?;

    Ok(QuorumScenario {
        node_ids,
        membership,
        partial: btreeset! {1, 2},
        write_quorum: btreeset! {1, 4},
        candidate: 4,
        read_quorum: btreeset! {3, 4},
        read_and_write_quorum: btreeset! {2, 3, 4},
    })
}

/// Voters 1 to 4 in the grid `[[1,2],[3,4]]`, and learner 5.
fn changes() -> Result<ChangeScenario<AdvConfig>> {
    let old = GridMembership::new(vec![vec![1, 2], vec![3, 4]], btreeset! {1, 2, 3, 4, 5})?;

    // Only a role changes, so every quorum stays.
    let roles = btreemap! {4 => NodeRole::ReceiveLogs};
    let direct = GridChange::default().with_roles(roles);

    // The old row {3, 4} misses the new write quorum {1, 5}.
    let joint = GridChange::grid(vec![vec![2, 1], vec![3, 5]]);

    Ok(ChangeScenario {
        node_ids: btreeset! {1, 2, 3, 4, 5},
        old,
        direct,
        direct_membership: "{configs: [Config { read: GridQuorumSet { grid: [[1, 2], [3, 4]], rule: FullRow }, \
                            write: GridQuorumSet { grid: [[1, 2], [3, 4]], rule: OnePerRow }, \
                            nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                            2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                            4: ((), NodeRole(ReceiveLogs)), 5: ((), NodeRole(ReceiveLogs))} }]}",
        joint,
        joint_with_learner: "{configs: [Config { read: GridQuorumSet { grid: [[1, 2], [3, 4]], rule: FullRow }, \
                             write: GridQuorumSet { grid: [[1, 2], [3, 4]], rule: OnePerRow }, \
                             nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             4: ((), NodeRole(ReceiveLogs)), 5: ((), NodeRole(ReceiveLogs))} }, Config { \
                             read: GridQuorumSet { grid: [[2, 1], [3, 5]], rule: FullRow }, write: GridQuorumSet { \
                             grid: [[2, 1], [3, 5]], rule: OnePerRow }, \
                             nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(ReceiveLogs)), \
                             5: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        joint_without_learner: "{configs: [Config { read: GridQuorumSet { grid: [[1, 2], [3, 4]], rule: FullRow }, \
                                write: GridQuorumSet { grid: [[1, 2], [3, 4]], rule: OnePerRow }, \
                                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                4: ((), NodeRole(ReceiveLogs)), 5: ((), NodeRole(ReceiveLogs))} }, \
                                Config { read: GridQuorumSet { grid: [[2, 1], [3, 5]], rule: FullRow }, \
                                write: GridQuorumSet { grid: [[2, 1], [3, 5]], rule: OnePerRow }, \
                                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                5: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        target_with_learner: "{configs: [Config { read: GridQuorumSet { grid: [[2, 1], [3, 5]], rule: FullRow }, \
                              write: GridQuorumSet { grid: [[2, 1], [3, 5]], rule: OnePerRow }, \
                              nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                              2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                              3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(ReceiveLogs)), \
                              5: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        target_without_learner: "{configs: [Config { read: GridQuorumSet { grid: [[2, 1], [3, 5]], rule: FullRow }, \
                                 write: GridQuorumSet { grid: [[2, 1], [3, 5]], rule: OnePerRow }, \
                                 nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                 3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                 5: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        removed: 4,
    })
}

/// With standard leader IDs, an entry commits once a write quorum stores it, and a new leader
/// needs votes of a read quorum that is also a write quorum.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn commit_waits_for_write_quorum_and_survives_leader_change_with_standard_leader_ids() -> Result<()> {
    square()?.run::<StdConfig>().await
}

/// With advanced leader IDs, an entry commits once a write quorum stores it, and a read quorum
/// elects a new leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn commit_waits_for_write_quorum_and_survives_leader_change_with_advanced_leader_ids() -> Result<()> {
    square()?.run::<AdvConfig>().await
}

/// A role change goes directly; replacing voter 4 with learner 5 passes through a joint config and
/// keeps voter 4 as a learner.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn changes_retain_removed_voter_as_learner() -> Result<()> {
    changes()?.run(true).await
}

/// The same changes without retaining drop voter 4.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn changes_drop_removed_voter() -> Result<()> {
    changes()?.run(false).await
}

/// Every node restarts with the joint config in effect, restores it, and leaves it for the
/// target.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn restart_restores_joint_config() -> Result<()> {
    changes()?.restart_in_joint().await
}
