use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::Node;
use openraft::NodeId;
use openraft::membership::NodeRole;
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

    /// Adds each node of `add_nodes` that `nodes` lacks, then removes each node of
    /// `remove_nodes`, so a node in both is removed.
    ///
    /// A node that `nodes` already holds keeps its metadata, so `nodes` must hold every node that
    /// the membership knows: adding a known node must not replace its metadata, such as its
    /// address.
    pub(crate) fn edit_nodes(&self, nodes: &mut BTreeMap<NID, N>) {
        for (node_id, node) in &self.add_nodes {
            if !nodes.contains_key(node_id) {
                nodes.insert(node_id.clone(), node.clone());
            }
        }

        for node_id in &self.remove_nodes {
            nodes.remove(node_id);
        }
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
