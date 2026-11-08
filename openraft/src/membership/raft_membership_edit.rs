use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;

use crate::base::OptionalSend;
use crate::errors::MembershipError;
use crate::membership::RaftMembership;

/// An operation that produces a target membership configuration.
///
/// [`RaftTypeConfig::MembershipEdit`] selects the edit independently of the membership type.
/// OpenRaft applies it to the current effective membership inside the Raft core, and builds
/// learner additions with this trait's `add_learner()` constructor.
/// [`Self::Error`] is the edit's native error. OpenRaft wraps user-defined errors with
/// [`RaftTypeConfig::ErrorSource`](crate::RaftTypeConfig::ErrorSource) when applying the edit.
///
/// [`RaftTypeConfig::MembershipEdit`]: crate::RaftTypeConfig::MembershipEdit
#[since(version = "0.10.0", change = "removed make_uniform")]
#[since(version = "0.10.0", change = "apply now takes retain to handle demoted voters")]
#[since(version = "0.10.0")]
pub trait RaftMembershipEdit<M>
where
    Self: Debug + Display + OptionalSend + Sized + 'static,
    M: RaftMembership,
{
    /// The native error for an application-specific rejection of this edit.
    #[since(version = "0.10.0")]
    type Error: Error + 'static;

    /// Returns the target configuration of this edit.
    ///
    /// OpenRaft validates the result, then appends it directly or first appends a joint
    /// configuration. After a joint step, OpenRaft appends this exact target, preserving its node
    /// removals and roles. With `retain = true`, the edit keeps demoted voters' nodes unless it
    /// explicitly removes them. With `retain = false`, the result drops each voter of
    /// `membership` that becomes a learner, as [`NodeRef::is_learner()`] defines it. For a batch,
    /// apply this rule after all edits, using the original membership and the final result.
    /// The edit must preserve every other known node unless it explicitly removes it.
    /// The target must have usable read and write quorums, resolve every quorum node ID, and
    /// satisfy the intersection requirements of [`RaftMembership::QuorumSet`]. OpenRaft checks
    /// these conditions before appending it.
    ///
    /// [`NodeRef::is_learner()`]: crate::membership::NodeRef::is_learner
    #[since(version = "0.10.0", change = "added the retain parameter")]
    #[since(version = "0.10.0")]
    fn apply(self, membership: &M, retain: bool) -> Result<M, MembershipError<M::NodeId, Self::Error>>;

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
    /// add_learner(4, node4).apply(&membership, true) == voters [{1,2,3}], learner 4 with node4
    /// add_learner(3, other_node).apply(&membership, true) == membership
    /// ```
    ///
    /// Voter `3` keeps its role and its node, because replacing the node of a known ID, such as
    /// its address, can break consensus.
    fn add_learner(node_id: M::NodeId, node: M::Node) -> Self;
}
