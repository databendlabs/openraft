use anyhow::Result;
use maplit::btreemap;
use maplit::btreeset;
use openraft::alias::AsyncRuntimeOf;
use openraft::membership::NodeRole;
use openraft_membership::HierarchicalChange;
use openraft_membership::HierarchicalMembership;
use openraft_membership::quorum::Group;

use crate::fixtures::custom_membership::ChangeScenario;
use crate::fixtures::custom_membership::QuorumScenario;
use crate::fixtures::ut_harness;

openraft::declare_raft_types!(
    StdConfig:
        D = String,
        R = (),
        Node = (),
        LeaderId = openraft::impls::leader_id_std::LeaderId<u64, u64>,
        Membership = HierarchicalMembership<u64, ()>,
        MembershipEdit = HierarchicalChange<Self::NodeId, Self::Node>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

openraft::declare_raft_types!(
    AdvConfig:
        D = String,
        R = (),
        Node = (),
        Membership = HierarchicalMembership<u64, ()>,
        MembershipEdit = HierarchicalChange<Self::NodeId, Self::Node>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

/// Three levels: the root holds group a = {1, 2}, group b and voter 6, and group b holds group
/// b1 = {3, 4} and voter 5. A write quorum reaches 2 of a, b and voter 6: a with both voters, b
/// with voter 5 and a voter of b1. A read quorum reaches 2 of them: a with one voter, b with voter
/// 5 or both voters of b1.
fn three_levels() -> Result<QuorumScenario<HierarchicalMembership<u64, ()>>> {
    let b = Group {
        quorum_size: 2,
        voters: btreeset! {5},
        groups: btreemap! {"b1".to_string() => Group::of_voters(1, [3, 4])},
    };
    let groups = Group {
        quorum_size: 2,
        voters: btreeset! {6},
        groups: btreemap! {"a".to_string() => Group::of_voters(2, [1, 2]), "b".to_string() => b},
    };
    let node_ids = btreeset! {1, 2, 3, 4, 5, 6};
    let membership = HierarchicalMembership::new(groups, node_ids.clone())?;

    Ok(QuorumScenario {
        node_ids,
        membership,
        partial: btreeset! {1, 5},
        write_quorum: btreeset! {1, 2, 3, 5},
        candidate: 5,
        read_quorum: btreeset! {5, 6},
        read_and_write_quorum: btreeset! {3, 5, 6},
    })
}

/// Groups a = {1, 2} and b = {3, 4}. A write quorum holds both voters of one group, so a read
/// quorum holds a voter of each group.
fn changes() -> Result<ChangeScenario<AdvConfig>> {
    let groups = Group::of_groups(1, [
        ("a", Group::of_voters(2, [1, 2])),
        ("b", Group::of_voters(2, [3, 4])),
    ]);
    let old = HierarchicalMembership::new(groups, btreeset! {1, 2, 3, 4})?;

    // Only a role changes, so every quorum stays.
    let roles = btreemap! {2 => NodeRole::ReceiveLogs};
    let direct = HierarchicalChange::default().with_roles(roles);

    // Voter 3 leaves group b, which then writes with voter 4 alone. The old read quorum {1, 3}
    // misses the new write quorum {4}.
    let groups = Group::of_groups(1, [("a", Group::of_voters(2, [1, 2])), ("b", Group::of_voters(1, [4]))]);
    let joint = HierarchicalChange::groups(groups);

    Ok(ChangeScenario {
        node_ids: btreeset! {1, 2, 3, 4},
        old,
        direct,
        direct_membership: "{configs: [Config { read: HierarchicalQuorumSet { root: Group { quorum_size: 2, \
                            voters: {}, groups: {\"a\": Group { quorum_size: 1, voters: {1, 2}, groups: {} }, \
                            \"b\": Group { quorum_size: 1, voters: {3, 4}, groups: {} }} } }, \
                            write: HierarchicalQuorumSet { root: Group { quorum_size: 1, voters: {}, \
                            groups: {\"a\": Group { quorum_size: 2, voters: {1, 2}, groups: {} }, \"b\": Group { \
                            quorum_size: 2, voters: {3, 4}, groups: {} }} } }, \
                            nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(ReceiveLogs)), \
                            3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                            4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        joint,
        joint_with_learner: "{configs: [Config { read: HierarchicalQuorumSet { root: Group { quorum_size: 2, \
                             voters: {}, groups: {\"a\": Group { quorum_size: 1, voters: {1, 2}, groups: {} }, \
                             \"b\": Group { quorum_size: 1, voters: {3, 4}, groups: {} }} } }, \
                             write: HierarchicalQuorumSet { root: Group { quorum_size: 1, voters: {}, \
                             groups: {\"a\": Group { quorum_size: 2, voters: {1, 2}, groups: {} }, \"b\": Group { \
                             quorum_size: 2, voters: {3, 4}, groups: {} }} } }, \
                             nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(ReceiveLogs)), \
                             3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             4: ((), NodeRole(Campaigner | ReceiveLogs))} }, Config { read: HierarchicalQuorumSet { \
                             root: Group { quorum_size: 2, voters: {}, groups: {\"a\": Group { quorum_size: 1, \
                             voters: {1, 2}, groups: {} }, \"b\": Group { quorum_size: 1, voters: {4}, \
                             groups: {} }} } }, write: HierarchicalQuorumSet { root: Group { quorum_size: 1, \
                             voters: {}, groups: {\"a\": Group { quorum_size: 2, voters: {1, 2}, groups: {} }, \
                             \"b\": Group { quorum_size: 1, voters: {4}, groups: {} }} } }, \
                             nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(ReceiveLogs)), \
                             3: ((), NodeRole(ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        joint_without_learner: "{configs: [Config { read: HierarchicalQuorumSet { root: Group { quorum_size: 2, \
                                voters: {}, groups: {\"a\": Group { quorum_size: 1, voters: {1, 2}, groups: {} }, \
                                \"b\": Group { quorum_size: 1, voters: {3, 4}, groups: {} }} } }, \
                                write: HierarchicalQuorumSet { root: Group { quorum_size: 1, voters: {}, \
                                groups: {\"a\": Group { quorum_size: 2, voters: {1, 2}, groups: {} }, \"b\": Group { \
                                quorum_size: 2, voters: {3, 4}, groups: {} }} } }, \
                                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(ReceiveLogs)), \
                                3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                4: ((), NodeRole(Campaigner | ReceiveLogs))} }, Config { read: HierarchicalQuorumSet { \
                                root: Group { quorum_size: 2, voters: {}, groups: {\"a\": Group { quorum_size: 1, \
                                voters: {1, 2}, groups: {} }, \"b\": Group { quorum_size: 1, voters: {4}, \
                                groups: {} }} } }, write: HierarchicalQuorumSet { root: Group { quorum_size: 1, \
                                voters: {}, groups: {\"a\": Group { quorum_size: 2, voters: {1, 2}, groups: {} }, \
                                \"b\": Group { quorum_size: 1, voters: {4}, groups: {} }} } }, \
                                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(ReceiveLogs)), \
                                4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        target_with_learner: "{configs: [Config { read: HierarchicalQuorumSet { root: Group { quorum_size: 2, \
                              voters: {}, groups: {\"a\": Group { quorum_size: 1, voters: {1, 2}, groups: {} }, \
                              \"b\": Group { quorum_size: 1, voters: {4}, groups: {} }} } }, \
                              write: HierarchicalQuorumSet { root: Group { quorum_size: 1, voters: {}, \
                              groups: {\"a\": Group { quorum_size: 2, voters: {1, 2}, groups: {} }, \"b\": Group { \
                              quorum_size: 1, voters: {4}, groups: {} }} } }, \
                              nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(ReceiveLogs)), \
                              3: ((), NodeRole(ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        target_without_learner: "{configs: [Config { read: HierarchicalQuorumSet { root: Group { quorum_size: 2, \
                                 voters: {}, groups: {\"a\": Group { quorum_size: 1, voters: {1, 2}, groups: {} }, \
                                 \"b\": Group { quorum_size: 1, voters: {4}, groups: {} }} } }, \
                                 write: HierarchicalQuorumSet { root: Group { quorum_size: 1, voters: {}, \
                                 groups: {\"a\": Group { quorum_size: 2, voters: {1, 2}, groups: {} }, \"b\": Group { \
                                 quorum_size: 1, voters: {4}, groups: {} }} } }, \
                                 nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(ReceiveLogs)), \
                                 4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        removed: 3,
    })
}

/// With standard leader IDs, an entry commits once a write quorum stores it, and a new leader
/// needs votes of a read quorum that is also a write quorum.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn commit_waits_for_write_quorum_and_survives_leader_change_with_standard_leader_ids() -> Result<()> {
    three_levels()?.run::<StdConfig>().await
}

/// With advanced leader IDs, an entry commits once a write quorum stores it, and a read quorum
/// elects a new leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn commit_waits_for_write_quorum_and_survives_leader_change_with_advanced_leader_ids() -> Result<()> {
    three_levels()?.run::<AdvConfig>().await
}

/// A role change goes directly; a change of the groups passes through a joint config and keeps
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
