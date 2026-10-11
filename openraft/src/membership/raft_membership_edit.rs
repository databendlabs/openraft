use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;

use crate::base::OptionalSend;
use crate::errors::MembershipError;
use crate::membership::RaftMembership;

/// An operation that produces a target membership configuration.
///
/// [`RaftTypeConfig::MembershipEdit`] selects the edit independently of the membership type.
/// OpenRaft applies it to the current effective membership inside the Raft core, and builds the
/// edits it needs itself with the constructors of this trait.
///
/// [`RaftTypeConfig::MembershipEdit`]: crate::RaftTypeConfig::MembershipEdit
#[since(version = "0.10.0", change = "parameterized by membership and added apply()")]
#[since(version = "0.10.0")]
pub trait RaftMembershipEdit<M>
where
    Self: Debug + Display + OptionalSend + Sized + 'static,
    M: RaftMembership,
{
    /// Returns the target configuration of this edit.
    ///
    /// OpenRaft validates the result, then appends it directly or first appends a joint
    /// configuration. With `retain = false`, removed voters that become learners are dropped.
    /// An edit must preserve known nodes unless it explicitly removes them.
    /// The target must have usable read and write quorums and resolve every quorum node ID.
    /// OpenRaft checks these conditions before appending it.
    #[since(version = "0.10.0")]
    fn apply(self, membership: &M) -> Result<M, MembershipError<M::NodeId>>;

    /// Returns an edit that adds `node_id` with `node` as a learner.
    ///
    /// [`Raft::add_learner()`](crate::Raft::add_learner) uses it.
    ///
    /// # Examples
    ///
    /// Suppose `membership` has voters `[{1,2,3}]` and no learner. Then
    /// [`apply()`](Self::apply) must return:
    ///
    /// ```text
    /// add_learner(4, node4).apply(&membership) == voters [{1,2,3}], learner 4 with node4
    /// add_learner(3, other_node).apply(&membership) == membership
    /// ```
    ///
    /// Voter `3` keeps its role and its node, because replacing the node of a known ID, such as
    /// its address, can break consensus.
    fn add_learner(node_id: M::NodeId, node: M::Node) -> Self;

    /// Returns an edit that reduces a joint configuration to its uniform target.
    ///
    /// Applied to a joint configuration, [`apply()`](Self::apply) must return
    /// the target of that joint configuration. The second step of
    /// [`Raft::change_membership()`](crate::Raft::change_membership) uses it.
    ///
    /// # Examples
    ///
    /// Suppose `membership` is the joint configuration `[{1,2,3}, {3,4,5}]` with no learner. Then
    /// `apply()` must return:
    ///
    /// ```text
    /// make_uniform().apply(&membership) == voters [{3,4,5}], learners {1,2}
    /// ```
    ///
    /// [`ChangeMembers`] returns `AddVoterIds` with an empty set, which keeps the target `{3,4,5}`
    /// as it is. Its `apply()` applies an edit to the target and returns the result, so this edit
    /// returns `[{3,4,5}]`.
    ///
    /// [`ChangeMembers`]: crate::impls::ChangeMembers
    fn make_uniform() -> Self;
}
