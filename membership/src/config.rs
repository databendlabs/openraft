use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::Node;
use openraft::NodeId;
use openraft::errors::NodeNotFound;
use openraft::errors::Operation;
use openraft::membership::NodeRole;
use openraft::membership::errors::WriteVoterWithoutReceiveLogs;
use openraft_macros::since;

use crate::errors::ConfigsError;
use crate::quorum::QuorumSet;

mod joint_config;

/// One config of a membership: its read rule, its write rule, and the nodes it holds, each with
/// its metadata and its role in this config.
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

    /// Every node that this config holds, with its role here. A node outside both rules is a
    /// learner of this config.
    nodes: BTreeMap<Q::Id, (N, NodeRole)>,
}

impl<N, Q> Config<N, Q>
where
    N: Node,
    Q: QuorumSet + Clone,
    Q::Id: NodeId,
{
    /// Builds a config with the rules `read` and `write` that holds the nodes `nodes`.
    ///
    /// A node of `roles` takes its role there. Every other node takes the default role for its
    /// status in this config: [`NodeRole::Campaigner`] `|` [`NodeRole::ReceiveLogs`] for a voter,
    /// an ID of either rule, and [`NodeRole::ReceiveLogs`] for a learner. A role of a node outside
    /// `nodes` is dropped.
    ///
    /// Returns [`ConfigsError::NodeNotFound`] for the first voter without a node. Otherwise it
    /// returns [`ConfigsError::WriteVoterWithoutReceiveLogs`] for the first write voter whose role
    /// lacks [`NodeRole::ReceiveLogs`].
    pub(crate) fn new(
        read: Q,
        write: Q,
        roles: BTreeMap<Q::Id, NodeRole>,
        nodes: BTreeMap<Q::Id, N>,
    ) -> Result<Self, ConfigsError<Q::Id>> {
        let mut config = Self {
            read,
            write,
            nodes: BTreeMap::new(),
        };

        let voters = config.voters();
        for (node_id, node) in nodes {
            let role = match roles.get(&node_id) {
                Some(role) => *role,
                None => default_role(&voters, &node_id),
            };
            config.nodes.insert(node_id, (node, role));
        }

        config.ensure_voters_have_nodes()?;
        config.ensure_write_voters_receive_logs()?;
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

    /// Returns the roles of this config that differ from the default, with each node of `roles`
    /// set to its role there.
    ///
    /// Returns [`NodeNotFound`] for the first node of `roles` that `nodes` lacks.
    pub(crate) fn edit_roles(
        &self,
        roles: BTreeMap<Q::Id, NodeRole>,
        nodes: &BTreeMap<Q::Id, N>,
    ) -> Result<BTreeMap<Q::Id, NodeRole>, NodeNotFound<Q::Id>> {
        let voters = self.voters();
        let mut edited = BTreeMap::new();
        for (node_id, (_, role)) in &self.nodes {
            if *role != default_role(&voters, node_id) {
                edited.insert(node_id.clone(), *role);
            }
        }

        for (node_id, role) in roles {
            if !nodes.contains_key(&node_id) {
                return Err(NodeNotFound::new(node_id, Operation::None));
            }
            edited.insert(node_id, role);
        }
        Ok(edited)
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

    /// Returns [`WriteVoterWithoutReceiveLogs`] for the first write voter whose role in this
    /// config lacks [`NodeRole::ReceiveLogs`].
    fn ensure_write_voters_receive_logs(&self) -> Result<(), WriteVoterWithoutReceiveLogs<Q::Id>> {
        for node_id in self.write.ids() {
            let Some((_, role)) = self.nodes.get(&node_id) else {
                continue;
            };
            if !role.contains(NodeRole::ReceiveLogs) {
                return Err(WriteVoterWithoutReceiveLogs { node_id });
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

/// Returns the role of `node_id` in a config with voters `voters` that sets no role for it.
fn default_role<NID>(voters: &BTreeSet<NID>, node_id: &NID) -> NodeRole
where NID: NodeId {
    if voters.contains(node_id) {
        NodeRole::Campaigner | NodeRole::ReceiveLogs
    } else {
        NodeRole::ReceiveLogs
    }
}

#[cfg(test)]
mod config_test;
