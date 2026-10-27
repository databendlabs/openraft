use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::Node;
use openraft::NodeId;
use openraft::errors::NodeNotFound;
use openraft::errors::Operation;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::errors::WriteVoterWithoutReceiveLogs;

use crate::errors::ConfigsError;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;

/// One config of a membership: its read rule, its write rule, and the nodes it holds, each with
/// its metadata and its role in this config.
///
/// A membership stores its configs oldest first. It holds more than one config only during a
/// joint change, and then its last config is the target of the change. A set of voters is a read
/// quorum of the membership when the read rule of every config accepts it, and the same holds for
/// write quorums.
///
/// The active nodes of a membership are the nodes of its last config and the voters of its
/// earlier configs. A learner that only an earlier config holds is not active: the change to the
/// last config removed it. An active node has its role in the last config, joined with its role
/// in each earlier config where it is a voter. A joint therefore keeps every capability of the old
/// voters, and selecting its last config restores the roles of the target.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound(serialize = "Q: serde::Serialize", deserialize = "Q: serde::Deserialize<'de>"))
)]
pub(crate) struct Config<NID, N, Q>
where
    NID: NodeId,
    N: Node,
    Q: QuorumSet<Id = NID>,
{
    read: Q,
    write: Q,

    /// Every node that this config holds, with its role here. A node outside both rules is a
    /// learner of this config.
    nodes: BTreeMap<NID, (N, NodeRole)>,
}

impl<NID, N, Q> Config<NID, N, Q>
where
    NID: NodeId,
    N: Node,
    Q: QuorumSet<Id = NID> + Clone,
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
        roles: BTreeMap<NID, NodeRole>,
        nodes: BTreeMap<NID, N>,
    ) -> Result<Self, ConfigsError<NID>> {
        let voters = voters_of(&read, &write);
        let mut config_nodes = BTreeMap::new();
        for (node_id, node) in nodes {
            let role = config_role(&roles, &voters, &node_id);
            config_nodes.insert(node_id, (node, role));
        }

        let config = Self {
            read,
            write,
            nodes: config_nodes,
        };
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

    /// Returns the read quorum set of a membership with `configs`: rule i is the read rule of
    /// config i.
    pub(crate) fn read_quorum_set(configs: &[Self]) -> Joint<Q> {
        let rules = configs.iter().map(|config| config.read.clone());
        Joint::new(rules)
    }

    /// Returns the write quorum set of a membership with `configs`: rule i is the write rule of
    /// config i.
    pub(crate) fn write_quorum_set(configs: &[Self]) -> Joint<Q> {
        let rules = configs.iter().map(|config| config.write.clone());
        Joint::new(rules)
    }

    /// Returns whether some config of `configs` and some config of `other` have the same read
    /// rule and the same write rule. The nodes and roles of the configs do not count, because they
    /// change no quorum.
    ///
    /// Such a shared config proves that every read quorum of each of `configs` and `other`
    /// intersects every write quorum of the other. A read quorum of `configs` satisfies the read
    /// rule of every config of `configs`, the shared one included. In the same way, a write quorum
    /// of `other` satisfies the write rule of the shared config. Every constructor and every change
    /// of a membership builds a config whose read rule intersects its write rule, so the two
    /// quorums share a voter. The reverse direction follows the same proof.
    ///
    /// A change of only roles or learners keeps both rules. A step into a joint keeps every config
    /// of the old membership, and a step out of a joint keeps every config of its goal. Every
    /// other change of a rule goes through a joint, even when every old quorum would meet every
    /// new one.
    pub(crate) fn supports_direct_transition(configs: &[Self], other: &[Self]) -> bool
    where Q: PartialEq {
        for config in configs {
            for other_config in other {
                let same_read = config.read == other_config.read;
                let same_write = config.write == other_config.write;
                if same_read && same_write {
                    return true;
                }
            }
        }
        false
    }

    /// Returns the metadata, the role and the voter flags of an active node of `configs`.
    ///
    /// The configs that count for the node are the last config and each earlier config where it
    /// is a voter. Its role joins its roles in these configs. Its metadata comes from the newest
    /// config that holds it, and its voter flags come from the rules of every config.
    pub(crate) fn get_node<'a>(configs: &'a [Self], node_id: &NID) -> Option<NodeRef<'a, N>> {
        let (last, earlier) = configs.split_last()?;

        let mut node = None;
        let mut role = NodeRole::Empty;
        let mut is_active = false;
        for config in earlier {
            let Some((config_node, config_role)) = config.nodes.get(node_id) else {
                continue;
            };
            node = Some(config_node);
            if config.is_voter(node_id) {
                role |= *config_role;
                is_active = true;
            }
        }
        if let Some((last_node, last_role)) = last.nodes.get(node_id) {
            node = Some(last_node);
            role |= *last_role;
            is_active = true;
        }

        let node = node?;
        if !is_active {
            return None;
        }
        let is_read_voter = configs.iter().any(|config| has_id(&config.read, node_id));
        let is_write_voter = configs.iter().any(|config| has_id(&config.write, node_id));
        let node_ref = NodeRef {
            node,
            role,
            is_read_voter,
            is_write_voter,
        };
        Some(node_ref)
    }

    /// Returns every active node ID of `configs` once, in ascending order: the nodes of the last
    /// config and the voters that the earlier configs hold.
    pub(crate) fn node_ids(configs: &[Self]) -> std::vec::IntoIter<NID> {
        let Some((last, earlier)) = configs.split_last() else {
            return Vec::new().into_iter();
        };

        let keys = last.nodes.keys();
        let mut ids: BTreeSet<NID> = keys.cloned().collect();
        for config in earlier {
            let voters = voters_of(&config.read, &config.write);
            let held = voters.into_iter().filter(|node_id| config.nodes.contains_key(node_id));
            ids.extend(held);
        }
        let ids: Vec<NID> = ids.into_iter().collect();
        ids.into_iter()
    }

    /// Returns the configs of the joint of `configs` and `goal`: the configs of `configs`
    /// followed by the configs of `goal`.
    ///
    /// Returns no config when either side has none: a membership without a config has no quorum,
    /// so a joint with it has no quorum either.
    pub(crate) fn joint(configs: &[Self], goal: &[Self]) -> Vec<Self> {
        if configs.is_empty() || goal.is_empty() {
            return Vec::new();
        }
        let mut joint = configs.to_vec();
        joint.extend_from_slice(goal);
        joint
    }

    /// Returns `goal` without the voters of `configs` that are learners of `goal`, as
    /// [`NodeRef::is_learner()`] defines them. Such a node leaves every config of `goal`.
    pub(crate) fn drop_voters_demoted_to_learners(configs: &[Self], mut goal: Vec<Self>) -> Vec<Self> {
        let mut demoted = Vec::new();
        for node_id in Self::node_ids(&goal) {
            // Safe unwrap(): `node_ids()` lists only the IDs that `get_node()` knows.
            let node = Self::get_node(&goal, &node_id).unwrap();
            let was_voter = configs.iter().any(|config| config.is_voter(&node_id));
            if was_voter && node.is_learner() {
                demoted.push(node_id);
            }
        }

        for config in &mut goal {
            for node_id in &demoted {
                config.nodes.remove(node_id);
            }
        }
        goal
    }

    /// Returns `configs` with each node of `roles` set to its role there in the last config. The
    /// other nodes keep their roles.
    ///
    /// Returns [`ConfigsError::NodeNotFound`] for the first node of `roles` that the last config
    /// does not hold. Otherwise it returns [`ConfigsError::WriteVoterWithoutReceiveLogs`] for the
    /// first write voter of the last config whose role lacks [`NodeRole::ReceiveLogs`].
    pub(crate) fn with_roles(
        mut configs: Vec<Self>,
        roles: BTreeMap<NID, NodeRole>,
    ) -> Result<Vec<Self>, ConfigsError<NID>> {
        for (node_id, role) in roles {
            let node = match configs.last_mut() {
                Some(last) => last.nodes.get_mut(&node_id),
                None => None,
            };
            let Some((_, node_role)) = node else {
                return Err(NodeNotFound::new(node_id, Operation::None).into());
            };
            *node_role = role;
        }

        if let Some(last) = configs.last() {
            last.ensure_write_voters_receive_logs()?;
        }
        Ok(configs)
    }

    /// Returns the nodes of the last of `configs` after adding `add_nodes` and then removing
    /// `remove_nodes`. An active node of `configs` keeps its metadata.
    pub(crate) fn edit_nodes(
        configs: &[Self],
        add_nodes: BTreeMap<NID, N>,
        remove_nodes: &BTreeSet<NID>,
    ) -> BTreeMap<NID, N> {
        let mut nodes = BTreeMap::new();
        if let Some(last) = configs.last() {
            for (node_id, (node, _)) in &last.nodes {
                nodes.insert(node_id.clone(), node.clone());
            }
        }
        for (node_id, node) in add_nodes {
            let known = Self::get_node(configs, &node_id);
            let node = match known {
                Some(known) => known.node.clone(),
                None => node,
            };
            nodes.insert(node_id, node);
        }
        for node_id in remove_nodes {
            nodes.remove(node_id);
        }
        nodes
    }

    /// Returns the roles of this config that differ from the default, with each node of `roles`
    /// set to its role there.
    ///
    /// Returns [`NodeNotFound`] for the first node of `roles` that `nodes` lacks.
    pub(crate) fn edit_roles(
        &self,
        roles: BTreeMap<NID, NodeRole>,
        nodes: &BTreeMap<NID, N>,
    ) -> Result<BTreeMap<NID, NodeRole>, NodeNotFound<NID>> {
        let voters = voters_of(&self.read, &self.write);
        let mut edited = BTreeMap::new();
        for (node_id, (_, role)) in &self.nodes {
            let is_default = *role == default_role(&voters, node_id);
            if !is_default {
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

    /// Returns whether `node_id` is an ID of the read rule or of the write rule.
    fn is_voter(&self, node_id: &NID) -> bool {
        let is_read_voter = has_id(&self.read, node_id);
        let is_write_voter = has_id(&self.write, node_id);
        is_read_voter || is_write_voter
    }

    /// Returns [`NodeNotFound`] for the first voter that this config does not hold.
    fn ensure_voters_have_nodes(&self) -> Result<(), NodeNotFound<NID>> {
        let voters = voters_of(&self.read, &self.write);
        for node_id in voters {
            if !self.nodes.contains_key(&node_id) {
                return Err(NodeNotFound::new(node_id, Operation::None));
            }
        }
        Ok(())
    }

    /// Returns [`WriteVoterWithoutReceiveLogs`] for the first write voter whose role in this
    /// config lacks [`NodeRole::ReceiveLogs`].
    fn ensure_write_voters_receive_logs(&self) -> Result<(), WriteVoterWithoutReceiveLogs<NID>> {
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

/// Returns the IDs of a read rule and a write rule.
fn voters_of<NID, Q>(read: &Q, write: &Q) -> BTreeSet<NID>
where
    NID: NodeId,
    Q: QuorumSet<Id = NID>,
{
    let mut voters: BTreeSet<NID> = read.ids().collect();
    voters.extend(write.ids());
    voters
}

/// Returns whether `node_id` is an ID of `rule`.
fn has_id<NID, Q>(rule: &Q, node_id: &NID) -> bool
where
    NID: NodeId,
    Q: QuorumSet<Id = NID>,
{
    let mut ids = rule.ids();
    ids.any(|id| id == *node_id)
}

/// Returns the role that a config with voters `voters` and roles `roles` gives `node_id`.
fn config_role<NID>(roles: &BTreeMap<NID, NodeRole>, voters: &BTreeSet<NID>, node_id: &NID) -> NodeRole
where NID: NodeId {
    let role = roles.get(node_id);
    match role {
        Some(role) => *role,
        None => default_role(voters, node_id),
    }
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
