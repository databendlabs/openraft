use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;

use crate::Node;
use crate::NodeId;
use crate::base::OptionalSend;

/// A change to a membership configuration, which [`RaftMembership::change()`] applies.
///
/// [`RaftMembership::Edit`] names the edit type of a membership implementation. OpenRaft builds
/// the edits it needs itself with the constructors of this trait.
///
/// [`RaftMembership::Edit`]: crate::membership::RaftMembership::Edit
/// [`RaftMembership::change()`]: crate::membership::RaftMembership::change
#[since(version = "0.10.0")]
pub trait RaftMembershipEdit
where Self: Debug + Display + OptionalSend + Sized + 'static
{
    /// The node identifier type.
    type NodeId: NodeId;

    /// The node metadata type, such as a network address.
    type Node: Node;

    /// Returns an edit that adds `node_id` with `node` as a learner.
    ///
    /// [`Raft::add_learner()`](crate::Raft::add_learner) uses it.
    ///
    /// # Examples
    ///
    /// Suppose `membership` has voters `[{1,2,3}]` and no learner. Then
    /// [`RaftMembership::change()`] must return:
    ///
    /// ```text
    /// membership.change(add_learner(4, node4)) == voters [{1,2,3}], learner 4 with node4
    /// membership.change(add_learner(3, other_node)) == membership
    /// ```
    ///
    /// Voter `3` keeps its role and its node, because replacing the node of a known ID, such as
    /// its address, can break consensus.
    ///
    /// [`RaftMembership::change()`]: crate::membership::RaftMembership::change
    fn add_learner(node_id: Self::NodeId, node: Self::Node) -> Self;

    /// Returns an edit that reduces a joint configuration to its uniform target.
    ///
    /// Applied to a joint configuration, this edit must make [`RaftMembership::change()`] return
    /// the target of that joint configuration. The second step of
    /// [`Raft::change_membership()`](crate::Raft::change_membership) uses it.
    ///
    /// # Examples
    ///
    /// Suppose `membership` is the joint configuration `[{1,2,3}, {3,4,5}]` with no learner. Then
    /// `change()` must return:
    ///
    /// ```text
    /// membership.change(make_uniform()) == voters [{3,4,5}], learners {1,2}
    /// ```
    ///
    /// [`ChangeMembers`] returns `AddVoterIds` with an empty set, which keeps the target `{3,4,5}`
    /// as it is. Its `change()` applies an edit to the target and returns the result, so this edit
    /// returns `[{3,4,5}]`.
    ///
    /// [`ChangeMembers`]: crate::ChangeMembers
    /// [`RaftMembership::change()`]: crate::membership::RaftMembership::change
    fn make_uniform() -> Self;
}
