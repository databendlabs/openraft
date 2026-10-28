use std::collections::BTreeSet;
use std::convert::Infallible;
use std::fmt;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::EntryPayload;
use openraft::SnapshotPolicy;
use openraft::alias::AsyncRuntimeOf;
use openraft::entry::RaftEntry;
use openraft::errors::MembershipError;
use openraft::impls::ChangeMembers;
use openraft::impls::Membership;
use openraft::membership::NodeRef;
use openraft::membership::QuorumIntersection;
use openraft::membership::QuorumSet;
use openraft::membership::RaftMembership;
use openraft::membership::RaftMembershipEdit;
use openraft::testing::log_id;
use serde::Deserialize;
use serde::Serialize;

use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::custom_membership::cluster_config;
use crate::fixtures::ut_harness;

openraft::declare_raft_types!(
    TypeConfig:
        D = String,
        R = (),
        Node = (),
        Membership = UnprovenMembership,
        MembershipEdit = ChangeMembers<u64, ()>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

/// The quorum rule of the default `Membership`, for which `intersects_with()` proves nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct UnprovenQuorums(Vec<BTreeSet<u64>>);

impl QuorumSet for UnprovenQuorums {
    type Id = u64;
    type Iter = <Vec<BTreeSet<u64>> as QuorumSet>::Iter;

    fn is_quorum<'a, I>(&self, ids: I) -> bool
    where I: Iterator<Item = &'a u64> + Clone {
        self.0.is_quorum(ids)
    }

    fn ids(&self) -> Self::Iter {
        self.0.ids()
    }
}

impl QuorumIntersection<UnprovenQuorums> for UnprovenQuorums {
    fn intersects_with(&self, _other: &UnprovenQuorums) -> Option<bool> {
        None
    }
}

/// The default `Membership` with [`UnprovenQuorums`] as its read and write quorum sets.
///
/// It answers the quorum questions itself, because `intersects_with()` proves nothing for it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct UnprovenMembership {
    membership: Membership<u64, ()>,
    quorums: UnprovenQuorums,
}

impl From<Membership<u64, ()>> for UnprovenMembership {
    fn from(membership: Membership<u64, ()>) -> Self {
        let configs = membership.get_joint_config().clone();
        let quorums = UnprovenQuorums(configs);
        Self { membership, quorums }
    }
}

impl fmt::Display for UnprovenMembership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.membership, f)
    }
}

impl RaftMembership for UnprovenMembership {
    type NodeId = u64;
    type Node = ();

    type QuorumSet = UnprovenQuorums;

    fn read_quorum_set(&self) -> UnprovenQuorums {
        self.quorums.clone()
    }

    fn write_quorum_set(&self) -> UnprovenQuorums {
        self.quorums.clone()
    }

    /// Read and write quorums are both majorities of every config, so they always intersect.
    fn read_write_quorums_intersect(&self) -> Option<bool> {
        Some(true)
    }

    /// Accepts a step only when every config of `other` is a config of `self`. Every config uses
    /// the majority rule, so configs with equal voter sets are equal.
    fn supports_direct_transition_to(&self, other: &Self) -> bool {
        let configs = &self.quorums.0;
        let mut other_configs = other.quorums.0.iter();
        other_configs.all(|config| configs.contains(config))
    }

    fn get_node(&self, node_id: &u64) -> Option<NodeRef<'_, ()>> {
        RaftMembership::get_node(&self.membership, node_id)
    }

    type Iter = <Membership<u64, ()> as RaftMembership>::Iter;

    fn node_ids(&self) -> Self::Iter {
        RaftMembership::node_ids(&self.membership)
    }

    fn drop_voters_demoted_to_learners(&self, goal: Self) -> Self {
        let membership = RaftMembership::drop_voters_demoted_to_learners(&self.membership, goal.membership);
        Self::from(membership)
    }

    fn joint_with(&self, goal: &Self) -> Self {
        let membership = RaftMembership::joint_with(&self.membership, &goal.membership);
        Self::from(membership)
    }
}

/// Applies each edit as it applies to the default `Membership`.
impl RaftMembershipEdit<UnprovenMembership> for ChangeMembers<u64, ()> {
    type Error = Infallible;

    fn apply(self, membership: &UnprovenMembership) -> Result<UnprovenMembership, MembershipError<u64, Infallible>> {
        let target = <Self as RaftMembershipEdit<Membership<u64, ()>>>::apply(self, &membership.membership)?;
        Ok(UnprovenMembership::from(target))
    }

    fn add_learner(node_id: u64, node: ()) -> Self {
        <Self as RaftMembershipEdit<Membership<u64, ()>>>::add_learner(node_id, node)
    }

    fn make_uniform() -> Self {
        <Self as RaftMembershipEdit<Membership<u64, ()>>>::make_uniform()
    }
}

/// `change_membership()` appends a joint membership and then its target, although
/// `intersects_with()` proves no step.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn change_membership_completes_with_unproven_intersection() -> Result<()> {
    let initial = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
    let initial = UnprovenMembership::from(initial);
    let cluster = TestCluster::<TypeConfig>::start(
        cluster_config(SnapshotPolicy::Never),
        btreeset! {1, 2, 3, 4},
        initial.clone(),
    )
    .await?;

    tracing::info!(
        node = 1,
        "--- elect the leader and commit its first log before changing the membership"
    );
    {
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(1).wait(timeout()).applied_index(Some(1), "commit leader blank").await?;
    }

    tracing::info!(
        log_index = 3,
        "--- make learner 4 a voter: no step is provable, so the change needs a joint membership"
    );
    let target = {
        let response = cluster.raft(1).change_membership(btreeset! {1, 2, 3, 4}, false).await?;
        let expected_log_id = log_id::<TypeConfig>(1, 1, 3);
        assert_eq!(expected_log_id, response.log_id);
        let target = Membership::new_with_defaults(vec![btreeset! {1, 2, 3, 4}], []);
        let target = UnprovenMembership::from(target);
        let expected_membership = Some(target.clone());
        assert_eq!(expected_membership, response.membership);
        target
    };

    tracing::info!(
        node = 1,
        "--- check that the applied history holds the joint membership and then its target"
    );
    {
        cluster.raft(1).wait(timeout()).applied_index(Some(3), "apply the target").await?;
        let joint = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}, btreeset! {1, 2, 3, 4}], []);
        let joint = UnprovenMembership::from(joint);
        let expected = vec![initial, joint, target];
        let applied = cluster.applied(1);
        let memberships = applied.iter().filter_map(|entry| entry.get_membership());
        let memberships: Vec<_> = memberships.collect();
        assert_eq!(expected, memberships);
    }

    Ok(())
}

/// `add_learner()` and `append_membership()` keep the voters, so each appends its membership
/// without a joint membership, although `intersects_with()` proves no step.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn keeping_voters_needs_no_joint_with_unproven_intersection() -> Result<()> {
    let initial = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
    let initial = UnprovenMembership::from(initial);
    let cluster = TestCluster::<TypeConfig>::start(
        cluster_config(SnapshotPolicy::Never),
        btreeset! {1, 2, 3, 4},
        initial.clone(),
    )
    .await?;

    tracing::info!(
        node = 1,
        "--- elect the leader and commit its first log before changing the membership"
    );
    {
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(1).wait(timeout()).applied_index(Some(1), "commit leader blank").await?;
    }

    tracing::info!(
        log_index = 2,
        "--- add learner 5: the voters stay, so the membership stays uniform"
    );
    {
        let response = cluster.raft(1).add_learner(5, (), false).await?;
        let expected_log_id = log_id::<TypeConfig>(1, 1, 2);
        assert_eq!(expected_log_id, response.log_id);
        let expected = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], [4, 5]);
        let expected_membership = Some(UnprovenMembership::from(expected));
        assert_eq!(expected_membership, response.membership);
    }

    tracing::info!(
        log_index = 3,
        "--- append the initial membership: it keeps the voters, so it is appended directly"
    );
    {
        let response = cluster.raft(1).append_membership(initial.clone(), EntryPayload::Blank, []).await?;
        let expected_log_id = log_id::<TypeConfig>(1, 1, 3);
        assert_eq!(expected_log_id, response.log_id);
        let expected_membership = Some(initial);
        assert_eq!(expected_membership, response.membership);
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
