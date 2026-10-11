use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;
use quorum_set::QuorumSet;

use crate::Node;
use crate::NodeId;
use crate::base::OptionalFeatures;
use crate::base::OptionalSend;
use crate::base::OptionalSync;
use crate::membership::NodeRef;
use crate::membership::NodeRole;

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
    type QuorumSet: QuorumSet<Id = Self::NodeId> + Clone + Debug + OptionalSend + OptionalSync + 'static;

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
}
