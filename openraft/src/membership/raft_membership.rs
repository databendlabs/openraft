use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;
use quorum_set::QuorumIntersection;
use quorum_set::QuorumSet;

use crate::Node;
use crate::NodeId;
use crate::base::OptionalFeatures;
use crate::base::OptionalSend;
use crate::base::OptionalSerde;
use crate::base::OptionalSync;
use crate::membership::NodeRef;

/// An immutable membership configuration with read and write quorum rules.
///
/// A configuration supplied to OpenRaft must satisfy these invariants, including after
/// deserialization:
///
/// - Every read quorum intersects every write quorum. Read quorums may be disjoint from other read
///   quorums, and write quorums may be disjoint from other write quorums.
/// - Every ID from either quorum set resolves through [`get_node()`](Self::get_node).
/// - A [`NodeRef`]'s voter flags match whether its ID appears in the corresponding quorum set's
///   [`ids()`](QuorumSet::ids) iterator.
/// - Every write voter has [`NodeRole::ReceiveLogs`].
/// - [`node_ids()`](Self::node_ids) lists each known node exactly once, including nodes outside
///   both quorum sets. [`get_node()`](Self::get_node) returns `Some` exactly for these IDs.
///
/// Quorum rules, node information, and capabilities must remain unchanged for the configuration's
/// lifetime. A membership change creates a new configuration.
///
/// [`NodeRole::ReceiveLogs`]: crate::membership::NodeRole::ReceiveLogs
#[since(version = "0.10.0")]
pub trait RaftMembership
where Self: OptionalFeatures + Clone + Default + Eq + Debug + Display + Sized + 'static
{
    /// The node identifier type.
    type NodeId: NodeId;

    /// The node metadata type, such as a network address.
    type Node: Node;

    /// The quorum rule type used for both reads and writes.
    ///
    /// OpenRaft accepts a configuration from the application only when
    /// [`read_write_quorums_intersect()`](Self::read_write_quorums_intersect) returns `Some(true)`.
    /// By default, it returns what [`intersects_with()`](QuorumIntersection::intersects_with)
    /// returns for the read quorum set against the write quorum set.
    ///
    /// OpenRaft appends a configuration directly after another, without a joint configuration,
    /// only when [`supports_direct_transition_to()`](Self::supports_direct_transition_to) returns
    /// `true`.
    ///
    /// When it cannot append the target of a change directly, OpenRaft first appends the
    /// [`joint_with()`](Self::joint_with) of the old and the new configuration. Every read quorum
    /// of it is a read quorum of both the old and the new configuration, and the same holds for
    /// write quorums. So on the step into it and on the step out of it, every read quorum of one
    /// side intersects every write quorum of the other side.
    type QuorumSet: QuorumSet<Id = Self::NodeId>
        + QuorumIntersection<Self::QuorumSet>
        + Clone
        + Debug
        + OptionalSend
        + OptionalSync
        + OptionalSerde
        + 'static;

    /// Returns the read quorum rule for this configuration.
    ///
    /// For a joint configuration, a read quorum must satisfy every component's read quorum rule.
    fn read_quorum_set(&self) -> Self::QuorumSet;

    /// Returns the write quorum rule for this configuration.
    ///
    /// For a joint configuration, a write quorum must satisfy every component's write quorum rule.
    fn write_quorum_set(&self) -> Self::QuorumSet;

    /// Returns whether every read quorum of this configuration intersects every write quorum.
    ///
    /// It returns `Some(true)` when the intersection is proven, `Some(false)` when it is
    /// disproven, and `None` when it is unknown. OpenRaft accepts a configuration from the
    /// application only when it returns `Some(true)`: the one passed to [`Raft::initialize()`] or
    /// [`Raft::append_membership()`], and the target that [`RaftMembershipEdit::apply()`] returns
    /// for [`Raft::change_membership()`].
    ///
    /// The default returns what [`intersects_with()`](QuorumIntersection::intersects_with) returns
    /// for the read quorum set against the write quorum set. Override it to return `Some(true)`
    /// when the quorum rule is valid but `intersects_with()` cannot prove it. OpenRaft trusts the
    /// answer, so a wrong `Some(true)` can lose committed entries.
    ///
    /// [`Raft::initialize()`]: crate::Raft::initialize
    /// [`Raft::append_membership()`]: crate::Raft::append_membership
    /// [`Raft::change_membership()`]: crate::Raft::change_membership
    /// [`RaftMembershipEdit::apply()`]: crate::membership::RaftMembershipEdit::apply
    #[since(version = "0.10.0")]
    fn read_write_quorums_intersect(&self) -> Option<bool> {
        let read = self.read_quorum_set();
        let write = self.write_quorum_set();
        read.intersects_with(&write)
    }

    /// Returns whether OpenRaft may append `other` right after `self`, as one log entry without a
    /// joint configuration.
    ///
    /// Return `true` only when every read quorum of `self` intersects every write quorum of
    /// `other`, and every write quorum of `self` intersects every read quorum of `other`. OpenRaft
    /// trusts the answer, so a wrong `true` can lose committed entries. `false` is always safe:
    /// OpenRaft then first appends the joint configuration of `self` and `other`.
    ///
    /// Return `true` at least when every config of `other` is also a config of `self`, with the
    /// same voter set and the same quorum rule. Then every read quorum of `self` is a read quorum
    /// of `other`, and every write quorum of `self` is a write quorum of `other`. Every read quorum
    /// of `other` intersects every write quorum of `other`, so the step is safe. The second step of
    /// [`Raft::change_membership()`] and [`Raft::add_learner()`] append such an `other`. If this
    /// method returns `false` for it, each of them appends another joint configuration instead.
    ///
    /// OpenRaft calls it only for configurations whose quorum sets reject the empty set and accept
    /// the set of all their voters. It also calls it only for an `other` for which
    /// [`read_write_quorums_intersect()`](Self::read_write_quorums_intersect) returned
    /// `Some(true)`.
    ///
    /// [`Raft::change_membership()`]: crate::Raft::change_membership
    /// [`Raft::add_learner()`]: crate::Raft::add_learner
    #[since(version = "0.10.0")]
    fn supports_direct_transition_to(&self, other: &Self) -> bool;

    /// Returns a known node's metadata, capabilities, and derived voter status.
    ///
    /// Returns `None` if `node_id` is absent from this configuration.
    fn get_node(&self, node_id: &Self::NodeId) -> Option<NodeRef<'_, Self::Node>>;

    /// An iterator over all known node IDs.
    type Iter: Iterator<Item = Self::NodeId>;

    /// Returns each known node ID exactly once, including nodes outside both quorum sets.
    fn node_ids(&self) -> Self::Iter;

    /// Returns `goal` without the voters of `self` that are learners of `goal`, as
    /// [`NodeRef::is_learner()`] defines them. Every other node of `goal` keeps its metadata and
    /// roles, and the quorum sets of `goal` stay.
    ///
    /// [`RaftMembershipEdit::apply()`](crate::membership::RaftMembershipEdit::apply) keeps a voter
    /// that an edit removes. OpenRaft calls this method when a membership change does not
    /// retain such voters as learners. A removed voter that keeps [`NodeRole::Campaigner`] in
    /// `goal` is not a learner, so it stays.
    ///
    /// [`NodeRole::Campaigner`]: crate::membership::NodeRole::Campaigner
    #[since(version = "0.10.0")]
    fn drop_voters_demoted_to_learners(&self, goal: Self) -> Self;

    /// Returns the joint configuration of `self` and `goal`.
    ///
    /// Every read quorum of it must be a read quorum of both `self` and `goal`, and every write
    /// quorum of it must be a write quorum of both. It knows every node of `goal` and every voter
    /// of `self`. A node takes its metadata from `goal` when `goal` knows it. A voter of `self`
    /// keeps the roles that it has in each config of `self` where it is a voter.
    ///
    /// OpenRaft appends the joint without checking it, so a joint with a quorum that misses a
    /// quorum of `self` or `goal` can lose committed entries.
    #[since(version = "0.10.0")]
    fn joint_with(&self, goal: &Self) -> Self;
}
