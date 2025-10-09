use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;

use maplit::btreemap;

use crate::EmptyNode;
use crate::Node;
use crate::NodeId;

/// Convert into a map of `Node`.
///
/// This is used as a user input acceptor when building a Membership, to convert various input types
/// into a map of `Node`.
pub trait IntoNodes<NID, N>
where
    N: Node,
    NID: NodeId,
{
    /// Convert this type into a map of node IDs to nodes.
    fn into_nodes(self) -> BTreeMap<NID, N>;
}

impl<NID, N> IntoNodes<NID, N> for ()
where
    N: Node,
    NID: NodeId,
{
    fn into_nodes(self) -> BTreeMap<NID, N> {
        btreemap! {}
    }
}

impl<NID> IntoNodes<NID, ()> for BTreeSet<NID>
where NID: NodeId
{
    fn into_nodes(self) -> BTreeMap<NID, ()> {
        self.into_iter().map(|node_id| (node_id, ())).collect()
    }
}

impl<NID> IntoNodes<NID, EmptyNode> for BTreeSet<NID>
where NID: NodeId
{
    fn into_nodes(self) -> BTreeMap<NID, EmptyNode> {
        self.into_iter().map(|node_id| (node_id, EmptyNode {})).collect()
    }
}

impl<NID, N> IntoNodes<NID, N> for BTreeMap<NID, N>
where
    N: Node,
    NID: NodeId,
{
    fn into_nodes(self) -> BTreeMap<NID, N> {
        self
    }
}

impl<NID, N> IntoNodes<NID, N> for HashMap<NID, N>
where
    N: Node,
    NID: NodeId,
{
    fn into_nodes(self) -> BTreeMap<NID, N> {
        self.into_iter().collect()
    }
}
