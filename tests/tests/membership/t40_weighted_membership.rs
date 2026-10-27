use anyhow::Result;
use maplit::btreemap;
use maplit::btreeset;
use openraft::alias::AsyncRuntimeOf;
use openraft::membership::NodeRole;
use openraft_membership::WeightedChange;
use openraft_membership::WeightedMembership;

use crate::fixtures::custom_membership::ChangeScenario;
use crate::fixtures::custom_membership::QuorumScenario;
use crate::fixtures::ut_harness;

openraft::declare_raft_types!(
    StdConfig:
        D = String,
        R = (),
        Node = (),
        LeaderId = openraft::impls::leader_id_std::LeaderId<u64, u64>,
        Membership = WeightedMembership<u64, ()>,
        MembershipEdit = WeightedChange<Self::NodeId, Self::Node>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

openraft::declare_raft_types!(
    AdvConfig:
        D = String,
        R = (),
        Node = (),
        Membership = WeightedMembership<u64, ()>,
        MembershipEdit = WeightedChange<Self::NodeId, Self::Node>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

/// Voter 1 weighs 2 and voters 2 to 5 weigh 1 each, 6 in total. A write quorum weighs 4, so a
/// read quorum weighs 3.
fn heavy_voter() -> Result<QuorumScenario<WeightedMembership<u64, ()>>> {
    let weights = btreemap! {1 => 2, 2 => 1, 3 => 1, 4 => 1, 5 => 1};
    let node_ids = btreeset! {1, 2, 3, 4, 5};
    let membership = WeightedMembership::new(weights, 4, node_ids.clone())?;

    Ok(QuorumScenario {
        node_ids,
        membership,
        partial: btreeset! {1, 2},
        write_quorum: btreeset! {1, 2, 3},
        candidate: 2,
        read_quorum: btreeset! {2, 3, 4},
        read_and_write_quorum: btreeset! {2, 3, 4, 5},
    })
}

/// Voters 1, 2 and 3 of weight 1 with write weight 2, and learner 4.
fn changes() -> Result<ChangeScenario<AdvConfig>> {
    let weights = btreemap! {1 => 1, 2 => 1, 3 => 1};
    let old = WeightedMembership::new(weights, 2, btreeset! {1, 2, 3, 4})?;

    // Only a role changes, so every quorum stays.
    let roles = btreemap! {3 => NodeRole::ReceiveLogs};
    let direct = WeightedChange::default().with_roles(roles);

    // The old read quorum {2, 3} misses the new write quorum {1, 4}.
    let weights = btreemap! {1 => 2, 2 => 1, 4 => 1};
    let joint = WeightedChange::weights(weights).with_write_weight(3);

    Ok(ChangeScenario {
        node_ids: btreeset! {1, 2, 3, 4},
        old,
        direct,
        direct_membership: "{configs: [Config { read: WeightedQuorumSet { weights: {1: 1, 2: 1, 3: 1}, \
                            threshold: 2 }, write: WeightedQuorumSet { weights: {1: 1, 2: 1, 3: 1}, threshold: 2 }, \
                            nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                            2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                            4: ((), NodeRole(ReceiveLogs))} }]}",
        joint,
        joint_with_learner: "{configs: [Config { read: WeightedQuorumSet { weights: {1: 1, 2: 1, 3: 1}, \
                             threshold: 2 }, write: WeightedQuorumSet { weights: {1: 1, 2: 1, 3: 1}, threshold: 2 }, \
                             nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                             4: ((), NodeRole(ReceiveLogs))} }, Config { read: WeightedQuorumSet { \
                             weights: {1: 2, 2: 1, 4: 1}, threshold: 2 }, write: WeightedQuorumSet { \
                             weights: {1: 2, 2: 1, 4: 1}, threshold: 3 }, \
                             nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                             4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        joint_without_learner: "{configs: [Config { read: WeightedQuorumSet { weights: {1: 1, 2: 1, 3: 1}, \
                                threshold: 2 }, write: WeightedQuorumSet { weights: {1: 1, 2: 1, 3: 1}, \
                                threshold: 2 }, nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                                4: ((), NodeRole(ReceiveLogs))} }, Config { read: WeightedQuorumSet { \
                                weights: {1: 2, 2: 1, 4: 1}, threshold: 2 }, write: WeightedQuorumSet { \
                                weights: {1: 2, 2: 1, 4: 1}, threshold: 3 }, \
                                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        target_with_learner: "{configs: [Config { read: WeightedQuorumSet { weights: {1: 2, 2: 1, 4: 1}, \
                              threshold: 2 }, write: WeightedQuorumSet { weights: {1: 2, 2: 1, 4: 1}, threshold: 3 }, \
                              nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                              2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                              4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        target_without_learner: "{configs: [Config { read: WeightedQuorumSet { weights: {1: 2, 2: 1, 4: 1}, \
                                 threshold: 2 }, write: WeightedQuorumSet { weights: {1: 2, 2: 1, 4: 1}, \
                                 threshold: 3 }, nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                 4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        removed: 3,
    })
}

/// With standard leader IDs, an entry commits once a write quorum stores it, and a new leader
/// needs votes of a read quorum that is also a write quorum.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn commit_waits_for_write_quorum_and_survives_leader_change_with_standard_leader_ids() -> Result<()> {
    heavy_voter()?.run::<StdConfig>().await
}

/// With advanced leader IDs, an entry commits once a write quorum stores it, and a read quorum
/// elects a new leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn commit_waits_for_write_quorum_and_survives_leader_change_with_advanced_leader_ids() -> Result<()> {
    heavy_voter()?.run::<AdvConfig>().await
}

/// A role change goes directly; a change of the weights passes through a joint config and keeps
/// voter 3 as a learner.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn changes_retain_removed_voter_as_learner() -> Result<()> {
    changes()?.run(true).await
}

/// The same changes without retaining drop voter 3.
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
