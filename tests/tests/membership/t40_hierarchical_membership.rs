use anyhow::Result;
use maplit::btreemap;
use maplit::btreeset;
use openraft::alias::AsyncRuntimeOf;
use openraft::membership::NodeRole;
use openraft_membership::HierarchicalChange;
use openraft_membership::HierarchicalMembership;

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

/// Groups a = {1, 2}, b = {3, 4} and c = {5, 6}. A write quorum holds both voters of 2 groups, so
/// a read quorum holds a voter of 2 groups.
fn three_groups() -> Result<QuorumScenario<HierarchicalMembership<u64, ()>>> {
    let groups = btreemap! {
        "a".to_string() => btreeset! {1, 2},
        "b".to_string() => btreeset! {3, 4},
        "c".to_string() => btreeset! {5, 6},
    };
    let write_sizes = btreemap! {"a".to_string() => 2, "b".to_string() => 2, "c".to_string() => 2};
    let node_ids = btreeset! {1, 2, 3, 4, 5, 6};
    let membership = HierarchicalMembership::new(groups, 2, write_sizes, node_ids.clone())?;

    Ok(QuorumScenario {
        node_ids,
        membership,
        partial: btreeset! {1, 3},
        write_quorum: btreeset! {1, 2, 3, 4},
        candidate: 3,
        read_quorum: btreeset! {3, 5},
        read_and_write_quorum: btreeset! {3, 4, 5, 6},
    })
}

/// Groups a = {1, 2} and b = {3, 4}. A write quorum holds both voters of one group, so a read
/// quorum holds a voter of each group.
fn changes() -> Result<ChangeScenario<AdvConfig>> {
    let groups = btreemap! {"a".to_string() => btreeset! {1, 2}, "b".to_string() => btreeset! {3, 4}};
    let write_sizes = btreemap! {"a".to_string() => 2, "b".to_string() => 2};
    let old = HierarchicalMembership::new(groups, 1, write_sizes, btreeset! {1, 2, 3, 4})?;

    // Only a role changes, so every quorum stays.
    let roles = btreemap! {2 => NodeRole::ReceiveLogs};
    let direct = HierarchicalChange::default().with_roles(roles);

    // Voter 3 leaves group b, which then writes with voter 4 alone. The old read quorum {1, 3}
    // misses the new write quorum {4}.
    let groups = btreemap! {"a".to_string() => btreeset! {1, 2}, "b".to_string() => btreeset! {4}};
    let write_sizes = btreemap! {"b".to_string() => 1};
    let joint = HierarchicalChange::groups(groups).with_write_sizes(write_sizes);

    Ok(ChangeScenario {
        node_ids: btreeset! {1, 2, 3, 4},
        old,
        direct,
        direct_membership: "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2}, \
                            quorum_size: 1 }, \"b\": Group { voters: {3, 4}, quorum_size: 1 }}, quorum_groups: 2 }, \
                            write: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2}, quorum_size: 2 }, \
                            \"b\": Group { voters: {3, 4}, quorum_size: 2 }}, quorum_groups: 1 }, \
                            nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(ReceiveLogs)), \
                            3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                            4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        joint,
        joint_with_learner: "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"a\": Group { \
                             voters: {1, 2}, quorum_size: 1 }, \"b\": Group { voters: {3, 4}, quorum_size: 1 }}, \
                             quorum_groups: 2 }, write: HierarchicalQuorumSet { groups: {\"a\": Group { \
                             voters: {1, 2}, quorum_size: 2 }, \"b\": Group { voters: {3, 4}, quorum_size: 2 }}, \
                             quorum_groups: 1 }, nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             2: ((), NodeRole(ReceiveLogs)), 3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             4: ((), NodeRole(Campaigner | ReceiveLogs))} }, Config { read: HierarchicalQuorumSet { \
                             groups: {\"a\": Group { voters: {1, 2}, quorum_size: 1 }, \"b\": Group { voters: {4}, \
                             quorum_size: 1 }}, quorum_groups: 2 }, write: HierarchicalQuorumSet { \
                             groups: {\"a\": Group { voters: {1, 2}, quorum_size: 2 }, \"b\": Group { voters: {4}, \
                             quorum_size: 1 }}, quorum_groups: 1 }, \
                             nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(ReceiveLogs)), \
                             3: ((), NodeRole(ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        joint_without_learner: "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"a\": Group { \
                                voters: {1, 2}, quorum_size: 1 }, \"b\": Group { voters: {3, 4}, quorum_size: 1 }}, \
                                quorum_groups: 2 }, write: HierarchicalQuorumSet { groups: {\"a\": Group { \
                                voters: {1, 2}, quorum_size: 2 }, \"b\": Group { voters: {3, 4}, quorum_size: 2 }}, \
                                quorum_groups: 1 }, nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                2: ((), NodeRole(ReceiveLogs)), 3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                4: ((), NodeRole(Campaigner | ReceiveLogs))} }, Config { \
                                read: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2}, \
                                quorum_size: 1 }, \"b\": Group { voters: {4}, quorum_size: 1 }}, quorum_groups: 2 }, \
                                write: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2}, \
                                quorum_size: 2 }, \"b\": Group { voters: {4}, quorum_size: 1 }}, quorum_groups: 1 }, \
                                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(ReceiveLogs)), \
                                4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        target_with_learner: "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"a\": Group { \
                              voters: {1, 2}, quorum_size: 1 }, \"b\": Group { voters: {4}, quorum_size: 1 }}, \
                              quorum_groups: 2 }, write: HierarchicalQuorumSet { groups: {\"a\": Group { \
                              voters: {1, 2}, quorum_size: 2 }, \"b\": Group { voters: {4}, quorum_size: 1 }}, \
                              quorum_groups: 1 }, nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                              2: ((), NodeRole(ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                              4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        target_without_learner: "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"a\": Group { \
                                 voters: {1, 2}, quorum_size: 1 }, \"b\": Group { voters: {4}, quorum_size: 1 }}, \
                                 quorum_groups: 2 }, write: HierarchicalQuorumSet { groups: {\"a\": Group { \
                                 voters: {1, 2}, quorum_size: 2 }, \"b\": Group { voters: {4}, quorum_size: 1 }}, \
                                 quorum_groups: 1 }, nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                 2: ((), NodeRole(ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        removed: 3,
    })
}

/// With standard leader IDs, an entry commits once a write quorum stores it, and a new leader
/// needs votes of a read quorum that is also a write quorum.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn commit_waits_for_write_quorum_and_survives_leader_change_with_standard_leader_ids() -> Result<()> {
    three_groups()?.run::<StdConfig>().await
}

/// With advanced leader IDs, an entry commits once a write quorum stores it, and a read quorum
/// elects a new leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn commit_waits_for_write_quorum_and_survives_leader_change_with_advanced_leader_ids() -> Result<()> {
    three_groups()?.run::<AdvConfig>().await
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
