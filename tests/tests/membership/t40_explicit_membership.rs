use anyhow::Result;
use maplit::btreemap;
use maplit::btreeset;
use openraft::alias::AsyncRuntimeOf;
use openraft::membership::NodeRole;
use openraft_membership::ExplicitChange;
use openraft_membership::ExplicitMembership;

use crate::fixtures::custom_membership::ChangeScenario;
use crate::fixtures::custom_membership::QuorumScenario;
use crate::fixtures::ut_harness;

openraft::declare_raft_types!(
    StdConfig:
        D = String,
        R = (),
        Node = (),
        LeaderId = openraft::impls::leader_id_std::LeaderId<u64, u64>,
        Membership = ExplicitMembership<u64, ()>,
        MembershipEdit = ExplicitChange<Self::NodeId, Self::Node>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

openraft::declare_raft_types!(
    AdvConfig:
        D = String,
        R = (),
        Node = (),
        Membership = ExplicitMembership<u64, ()>,
        MembershipEdit = ExplicitChange<Self::NodeId, Self::Node>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

/// Voters 1 and 2 form one site, and voters 3 and 4 another. A write quorum holds both voters of
/// a site, and a read quorum holds a voter of each site.
fn two_sites() -> Result<QuorumScenario<ExplicitMembership<u64, ()>>> {
    let read = [btreeset! {1, 3}, btreeset! {1, 4}, btreeset! {2, 3}, btreeset! {2, 4}];
    let write = [btreeset! {1, 2}, btreeset! {3, 4}];
    let node_ids = btreeset! {1, 2, 3, 4};
    let membership = ExplicitMembership::new(read, write, node_ids.clone())?;

    Ok(QuorumScenario {
        node_ids,
        membership,
        partial: btreeset! {1, 3},
        write_quorum: btreeset! {1, 2},
        candidate: 2,
        read_quorum: btreeset! {2, 4},
        read_and_write_quorum: btreeset! {2, 3, 4},
    })
}

/// Voters 1, 2 and 3 with every pair as a read quorum and as a write quorum, and learner 4.
fn changes() -> Result<ChangeScenario<AdvConfig>> {
    let pairs = [btreeset! {1, 2}, btreeset! {1, 3}, btreeset! {2, 3}];
    let old = ExplicitMembership::new(pairs.clone(), pairs, btreeset! {1, 2, 3, 4})?;

    // Only a role changes, so every quorum stays.
    let roles = btreemap! {3 => NodeRole::ReceiveLogs};
    let direct = ExplicitChange::default().with_roles(roles);

    // The new read quorum {1} misses the old write quorum {2, 3}.
    let read = vec![btreeset! {1}, btreeset! {2}, btreeset! {4}];
    let joint = ExplicitChange::quorums(read, vec![btreeset! {1, 2, 4}]);

    Ok(ChangeScenario {
        node_ids: btreeset! {1, 2, 3, 4},
        old,
        direct,
        direct_membership: "{configs: [Config { read: ExplicitQuorumSet { quorums: [{1, 2}, {1, 3}, {2, 3}] }, \
                            write: ExplicitQuorumSet { quorums: [{1, 2}, {1, 3}, {2, 3}] }, \
                            nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                            2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                            4: ((), NodeRole(ReceiveLogs))} }]}",
        joint,
        joint_with_learner: "{configs: [Config { read: ExplicitQuorumSet { quorums: [{1, 2}, {1, 3}, {2, 3}] }, \
                             write: ExplicitQuorumSet { quorums: [{1, 2}, {1, 3}, {2, 3}] }, \
                             nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             3: ((), NodeRole(ReceiveLogs)), \
                             4: ((), NodeRole(ReceiveLogs))} }, Config { read: ExplicitQuorumSet { \
                             quorums: [{1}, {2}, {4}] }, write: ExplicitQuorumSet { quorums: [{1, 2, 4}] }, \
                             nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                             2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                             4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        joint_without_learner: "{configs: [Config { read: ExplicitQuorumSet { quorums: [{1, 2}, {1, 3}, {2, 3}] }, \
                                write: ExplicitQuorumSet { quorums: [{1, 2}, {1, 3}, {2, 3}] }, \
                                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                3: ((), NodeRole(ReceiveLogs)), \
                                4: ((), NodeRole(ReceiveLogs))} }, Config { read: ExplicitQuorumSet { \
                                quorums: [{1}, {2}, {4}] }, write: ExplicitQuorumSet { quorums: [{1, 2, 4}] }, \
                                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                                4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        target_with_learner: "{configs: [Config { read: ExplicitQuorumSet { quorums: [{1}, {2}, {4}] }, \
                              write: ExplicitQuorumSet { quorums: [{1, 2, 4}] }, \
                              nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                              2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                              4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}",
        target_without_learner: "{configs: [Config { read: ExplicitQuorumSet { quorums: [{1}, {2}, {4}] }, \
                                 write: ExplicitQuorumSet { quorums: [{1, 2, 4}] }, \
                                 nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
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
    two_sites()?.run::<StdConfig>().await
}

/// With advanced leader IDs, an entry commits once a write quorum stores it, and a read quorum
/// elects a new leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn commit_waits_for_write_quorum_and_survives_leader_change_with_advanced_leader_ids() -> Result<()> {
    two_sites()?.run::<AdvConfig>().await
}

/// A role change goes directly; a change to single-voter read quorums passes through a joint
/// config and keeps voter 3 as a learner.
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
