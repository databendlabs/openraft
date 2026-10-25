use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;
use quorum_set::QuorumBridge;
use quorum_set::QuorumIntersection;
use quorum_set::QuorumSet;

use crate::Node;
use crate::NodeId;
use crate::base::OptionalFeatures;
use crate::base::OptionalSend;
use crate::base::OptionalSync;
use crate::errors::MembershipError;
use crate::membership::NodeRef;
use crate::membership::NodeRole;
use crate::membership::RaftMembershipEdit;

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
    /// [`intersects_with()`](QuorumIntersection::intersects_with) returns `Some(true)` for its read
    /// quorum set against its write quorum set. With
    /// [`Config::allow_unproven_read_write_intersection`] set, it also accepts `None`.
    ///
    /// OpenRaft appends a configuration directly after another, without a joint configuration,
    /// only when [`intersects_with()`](QuorumIntersection::intersects_with) returns `Some(true)`
    /// for the old read quorum set against the new write quorum set, and for the old write quorum
    /// set against the new read quorum set. Returning `None` is always safe.
    ///
    /// When it cannot append the target of a change directly, OpenRaft first appends a joint
    /// configuration. Its read quorum set is the [`joint_with()`](QuorumBridge::joint_with) of the
    /// old and the new read quorum sets, and its write quorum set comes from the write quorum sets
    /// the same way. Every read quorum of it is a read quorum of both the old and the new
    /// configuration, and the same holds for write quorums. So on the step into it and on the step
    /// out of it, every read quorum of one side intersects every write quorum of the other side.
    ///
    /// `intersects_with()` must return `Some(true)` for the joint read quorum set against the new
    /// write quorum set, and for the joint write quorum set against the new read quorum set.
    /// Otherwise the second step of [`Raft::change_membership()`](crate::Raft::change_membership)
    /// appends a second joint configuration instead of the new one.
    ///
    /// [`Config::allow_unproven_read_write_intersection`]: crate::Config::allow_unproven_read_write_intersection
    type QuorumSet: QuorumSet<Id = Self::NodeId>
        + QuorumIntersection<Self::QuorumSet>
        + QuorumBridge<Self::QuorumSet>
        + Clone
        + Debug
        + OptionalSend
        + OptionalSync
        + 'static;

    /// Returns the read quorum rule for this configuration.
    ///
    /// For a joint configuration, a read quorum must satisfy every component's read quorum rule.
    fn read_quorum_set(&self) -> &Self::QuorumSet;

    /// Returns the write quorum rule for this configuration.
    ///
    /// For a joint configuration, a write quorum must satisfy every component's write quorum rule.
    fn write_quorum_set(&self) -> &Self::QuorumSet;

    /// Returns a known node's metadata, capabilities, and derived voter status.
    ///
    /// Returns `None` if `node_id` is absent from this configuration.
    fn get_node(&self, node_id: &Self::NodeId) -> Option<NodeRef<'_, Self::Node>>;

    /// An iterator over all known node IDs.
    type Iter: Iterator<Item = Self::NodeId>;

    /// Returns each known node ID exactly once, including nodes outside both quorum sets.
    fn node_ids(&self) -> Self::Iter;

    /// Creates a configuration from validated quorum rules and node metadata with capabilities.
    ///
    /// Voter status is derived from the quorum sets.
    ///
    /// # Preconditions
    ///
    /// The caller must ensure every read quorum intersects every write quorum, every quorum node ID
    /// is present in `nodes`, and every write voter has [`NodeRole::ReceiveLogs`].
    #[since(version = "0.10.0")]
    fn new<I>(read_quorum_set: Self::QuorumSet, write_quorum_set: Self::QuorumSet, nodes: I) -> Self
    where I: IntoIterator<Item = (Self::NodeId, Self::Node, NodeRole)>;

    /// The edit type that [`change()`](Self::change) applies.
    #[since(version = "0.10.0")]
    type Edit: RaftMembershipEdit<NodeId = Self::NodeId, Node = Self::Node>;

    /// Returns the target configuration of `change`.
    ///
    /// OpenRaft appends the target directly, or first appends a joint configuration that it builds
    /// with [`joint_with()`](QuorumBridge::joint_with). The edit from
    /// [`RaftMembershipEdit::make_uniform()`] on a joint configuration must return its target.
    ///
    /// A node stays in the target unless `change` removes it, so a voter that `change` removes
    /// from the quorum sets stays with the role that the target gives it. When
    /// [`Raft::change_membership()`](crate::Raft::change_membership) gets `retain = false`,
    /// OpenRaft drops each such voter that the target makes a learner.
    ///
    /// OpenRaft rejects the result when its read or its write quorum set has no usable quorum,
    /// when a quorum ID has no node, or when OpenRaft cannot prove that its read quorums intersect
    /// its write quorums, as [`QuorumSet`](Self::QuorumSet) describes.
    #[since(version = "0.10.0")]
    fn change(&self, change: Self::Edit) -> Result<Self, MembershipError<Self::NodeId>>;
}
