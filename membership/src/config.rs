use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::Node;
use openraft::NodeId;
use openraft::errors::NodeNotFound;
use openraft::errors::Operation;
use openraft::membership::NodeRole;
use openraft_macros::since;

use crate::quorum::QuorumSet;

mod joint_config;

/// One config of a membership: its read rule, its write rule, and the nodes it holds, each with its
/// metadata and the role set for it, if any.
///
/// A node without a set role takes the default role for its status in this config:
/// [`NodeRole::Campaigner`] `|` [`NodeRole::ReceiveLogs`] for a voter, an ID of either rule, and
/// [`NodeRole::ReceiveLogs`] for a learner. The default is computed each time the role is read,
/// so it follows the status of the node.
///
/// A membership is a [`Joint`] of configs, oldest first. It holds more than one config only during
/// a joint change, and then its last config is the target of the change. [`ExplicitMembership`],
/// [`GridMembership`], [`HierarchicalMembership`] and [`WeightedMembership`] are such joints, each
/// with its own quorum rule.
///
/// [`Joint`]: crate::quorum::Joint
/// [`ExplicitMembership`]: crate::ExplicitMembership
/// [`GridMembership`]: crate::GridMembership
/// [`HierarchicalMembership`]: crate::HierarchicalMembership
/// [`WeightedMembership`]: crate::WeightedMembership
#[since(version = "0.10.0", change = "stores a role only for a node that has one set")]
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound(serialize = "Q: serde::Serialize", deserialize = "Q: serde::Deserialize<'de>"))
)]
pub struct Config<N, Q>
where
    N: Node,
    Q: QuorumSet,
    Q::Id: NodeId,
{
    read: Q,
    write: Q,

    /// Every node that this config holds, with its metadata and its set role. A node with no set
    /// role, `None`, takes its default role. A node outside both rules is a learner of this
    /// config.
    nodes: BTreeMap<Q::Id, (N, Option<NodeRole>)>,
}

impl<N, Q> Config<N, Q>
where
    N: Node,
    Q: QuorumSet + Clone,
    Q::Id: NodeId,
{
    /// Builds a config with the rules `read` and `write` that holds the nodes `nodes`, and sets no
    /// role.
    ///
    /// Returns [`NodeNotFound`] for the first voter without a node.
    pub(crate) fn new(read: Q, write: Q, nodes: BTreeMap<Q::Id, N>) -> Result<Self, NodeNotFound<Q::Id>> {
        let mut config_nodes = BTreeMap::new();
        for (node_id, node) in nodes {
            config_nodes.insert(node_id, (node, None));
        }

        let config = Self {
            read,
            write,
            nodes: config_nodes,
        };
        config.ensure_voters_have_nodes()?;
        Ok(config)
    }

    /// Returns the read rule.
    pub(crate) fn read(&self) -> &Q {
        &self.read
    }

    /// Returns the write rule.
    pub(crate) fn write(&self) -> &Q {
        &self.write
    }

    /// Sets each node of `roles` to its role there. A role equal to the default for the node's
    /// status in this config is not stored, so the node follows its default again.
    ///
    /// A role is kept as given, even one without [`NodeRole::ReceiveLogs`] for a write voter:
    /// OpenRaft rejects such a membership when it is initialized or proposed.
    ///
    /// Returns [`NodeNotFound`] for the first node of `roles` that this config does not hold, and
    /// then sets no role.
    pub(crate) fn set_roles(&mut self, roles: BTreeMap<Q::Id, NodeRole>) -> Result<(), NodeNotFound<Q::Id>> {
        for node_id in roles.keys() {
            if !self.nodes.contains_key(node_id) {
                return Err(NodeNotFound::new(node_id.clone(), Operation::None));
            }
        }

        for (node_id, role) in roles {
            let default = default_role(self.is_voter(&node_id));
            let set_role = if role == default { None } else { Some(role) };
            // Safe unwrap(): the loop above checked that this config holds every node of `roles`.
            let (_, node_role) = self.nodes.get_mut(&node_id).unwrap();
            *node_role = set_role;
        }
        Ok(())
    }

    /// Copies the roles that `other` sets for the nodes that this config holds, as they are: a
    /// set role stays until a change sets another one.
    pub(crate) fn keep_roles(&mut self, other: &Self) {
        for (node_id, (_, role)) in &mut self.nodes {
            if let Some((_, Some(other_role))) = other.nodes.get(node_id) {
                *role = Some(*other_role);
            }
        }
    }

    /// Returns the voters of this config: the IDs of its read rule and of its write rule.
    fn voters(&self) -> BTreeSet<Q::Id> {
        let mut voters: BTreeSet<Q::Id> = self.read.ids().collect();
        voters.extend(self.write.ids());
        voters
    }

    /// Returns whether `node_id` is an ID of the read rule or of the write rule.
    fn is_voter(&self, node_id: &Q::Id) -> bool {
        has_id(&self.read, node_id) || has_id(&self.write, node_id)
    }

    /// Returns [`NodeNotFound`] for the first voter that this config does not hold.
    fn ensure_voters_have_nodes(&self) -> Result<(), NodeNotFound<Q::Id>> {
        for node_id in self.voters() {
            if !self.nodes.contains_key(&node_id) {
                return Err(NodeNotFound::new(node_id, Operation::None));
            }
        }
        Ok(())
    }
}

/// Returns whether `node_id` is an ID of `rule`.
fn has_id<Q>(rule: &Q, node_id: &Q::Id) -> bool
where
    Q: QuorumSet,
    Q::Id: NodeId,
{
    rule.ids().any(|id| id == *node_id)
}

/// Returns the role of a node that its config sets no role for: [`NodeRole::Campaigner`] `|`
/// [`NodeRole::ReceiveLogs`] for a voter, and [`NodeRole::ReceiveLogs`] for a learner.
pub(crate) fn default_role(is_voter: bool) -> NodeRole {
    if is_voter {
        NodeRole::Campaigner | NodeRole::ReceiveLogs
    } else {
        NodeRole::ReceiveLogs
    }
}

#[cfg(test)]
mod config_test;
