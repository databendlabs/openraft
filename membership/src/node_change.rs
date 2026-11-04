use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::btree_map::Entry;

use openraft::Node;
use openraft::NodeId;
use openraft::membership::NodeRole;
use openraft::membership::RaftMembership;
use openraft_macros::since;

/// Node additions, removals and role assignments shared by membership edits.
///
/// Used by [`ExplicitChange`](crate::ExplicitChange), [`GridChange`](crate::GridChange),
/// [`HierarchicalChange`](crate::HierarchicalChange) and [`WeightedChange`](crate::WeightedChange).
///
/// An edit adds nodes, removes nodes, checks that every voter has metadata, then sets roles.
/// Adding an active node keeps its existing metadata. Removing an added node in the same edit
/// removes it from the result. Roles are applied before deciding whether to retain demoted voters.
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Nodes to add. A node that already exists keeps its metadata.
    pub add_nodes: BTreeMap<NID, N>,

    /// Nodes to remove. Removing a voter of the result fails.
    pub remove_nodes: BTreeSet<NID>,

    /// Roles to set for nodes of the result. Other nodes keep their explicitly set roles from
    /// the last config. Setting a node to its default role clears its stored role.
    pub roles: BTreeMap<NID, NodeRole>,
}

impl<NID, N> NodeChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns an edit that adds one node.
    #[since(version = "0.10.0")]
    pub fn add_node(node_id: NID, node: N) -> Self {
        let add_nodes = BTreeMap::from([(node_id, node)]);
        Self {
            add_nodes,
            ..Self::default()
        }
    }

    /// Returns this edit with `roles` as the roles it sets.
    #[since(version = "0.10.0")]
    pub fn with_roles(self, roles: BTreeMap<NID, NodeRole>) -> Self {
        Self { roles, ..self }
    }
}

impl<NID, N> Default for NodeChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn default() -> Self {
        Self {
            add_nodes: BTreeMap::new(),
            remove_nodes: BTreeSet::new(),
            roles: BTreeMap::new(),
        }
    }
}

/// Adds missing nodes, using the membership's metadata for an active node.
pub(crate) fn add_nodes_to<M>(
    nodes: &mut BTreeMap<M::NodeId, M::Node>,
    add_nodes: BTreeMap<M::NodeId, M::Node>,
    membership: &M,
) where
    M: RaftMembership,
{
    for (node_id, node) in add_nodes {
        let Entry::Vacant(entry) = nodes.entry(node_id) else {
            continue;
        };
        let node_id = entry.key();
        let node = match membership.get_node(node_id) {
            Some(known) => known.node.clone(),
            None => node,
        };
        entry.insert(node);
    }
}

/// Removes every node named in `remove_nodes`.
pub(crate) fn remove_nodes_from<NID, N>(nodes: &mut BTreeMap<NID, N>, remove_nodes: &BTreeSet<NID>)
where
    NID: NodeId,
    N: Node,
{
    for node_id in remove_nodes {
        nodes.remove(node_id);
    }
}
