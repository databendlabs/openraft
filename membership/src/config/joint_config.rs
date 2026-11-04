use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Debug;

use openraft::Node;
use openraft::NodeId;
use openraft::base::OptionalFeatures;
use openraft::errors::NodeNotFound;
use openraft::errors::Operation;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::RaftMembership;

use crate::config::Config;
use crate::config::default_role;
use crate::config::has_id;
use crate::quorum::Joint;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;

/// The edits that the membership changes of this crate apply to a joint of configs.
impl<N, Q> Joint<Config<N, Q>>
where
    N: Node,
    Q: QuorumSet + QuorumIntersection<Q> + Clone + Debug + Eq + OptionalFeatures + 'static,
    Q::Id: NodeId,
{
    /// Returns this membership with each node of `roles` set to its role there in the last config,
    /// as [`Config::set_roles()`] does. The other nodes keep their roles.
    ///
    /// Returns [`NodeNotFound`] for the first node of `roles` that the last config does not hold.
    pub(crate) fn set_roles(mut self, roles: BTreeMap<Q::Id, NodeRole>) -> Result<Self, NodeNotFound<Q::Id>> {
        let Some(last) = self.parts.last_mut() else {
            // A membership without a config holds no node.
            if let Some(node_id) = roles.into_keys().next() {
                return Err(NodeNotFound::new(node_id, Operation::None));
            }
            return Ok(self);
        };
        last.set_roles(roles)?;
        Ok(self)
    }

    /// Returns the nodes of the last config after adding `add_nodes` and then removing
    /// `remove_nodes`. An active node of this membership keeps its metadata.
    pub(crate) fn edit_nodes(
        &self,
        add_nodes: BTreeMap<Q::Id, N>,
        remove_nodes: &BTreeSet<Q::Id>,
    ) -> BTreeMap<Q::Id, N> {
        let mut nodes = BTreeMap::new();
        if let Some(last) = self.parts.last() {
            for (node_id, (node, _)) in &last.nodes {
                nodes.insert(node_id.clone(), node.clone());
            }
        }
        for (node_id, node) in add_nodes {
            let node = match self.get_node(&node_id) {
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

    /// Returns `goal` without the voters of this membership that are learners of `goal`, as
    /// [`NodeRef::is_learner()`] defines them. Such a node leaves every config of `goal`.
    pub(crate) fn drop_voters_demoted_to_learners(&self, mut goal: Self) -> Self {
        let mut demoted = BTreeSet::new();
        for node_id in goal.node_ids() {
            let was_voter = self.parts.iter().any(|config| config.is_voter(&node_id));
            if !was_voter {
                continue;
            }
            // Safe unwrap(): `node_ids()` lists only the IDs that `get_node()` knows.
            let node = goal.get_node(&node_id).unwrap();
            if node.is_learner() {
                demoted.insert(node_id);
            }
        }

        for config in &mut goal.parts {
            config.nodes.retain(|node_id, _| !demoted.contains(node_id));
        }
        goal
    }
}

/// Prints the configs, oldest first, such as `{configs: [Config { .. }]}`.
impl<N, Q> fmt::Display for Joint<Config<N, Q>>
where
    N: Node,
    Q: QuorumSet + Debug,
    Q::Id: NodeId,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{configs: {:?}}}", self.parts)
    }
}

/// A joint of configs is a membership: one config, or the old configs and the new one during a
/// joint change. Rule i of each quorum set belongs to config i.
///
/// The active nodes of a membership are the nodes of its last config and the voters of its
/// earlier configs. A learner that only an earlier config holds is not active: the change to the
/// last config removed it. An active node has its role in the last config, joined with its role
/// in each earlier config where it is a voter. A joint therefore keeps every capability of the old
/// voters, and selecting its last config restores the roles of the target.
///
/// Only voters carry over because the quorums of every config still apply during a joint, and
/// they count the old voters. A learner is in no quorum, so the last config decides it at once, as
/// it does for a change that touches only learners. This is the joint that
/// [`RaftMembership::joint_with()`] specifies.
impl<N, Q> RaftMembership for Joint<Config<N, Q>>
where
    N: Node,
    Q: QuorumSet + QuorumIntersection<Q> + Clone + Debug + Eq + OptionalFeatures + 'static,
    Q::Id: NodeId,
{
    type NodeId = Q::Id;
    type Node = N;
    type QuorumSet = Joint<Q>;

    fn read_quorum_set(&self) -> Self::QuorumSet {
        let rules = self.parts.iter().map(|config| config.read.clone());
        Joint::from_rules(rules)
    }

    fn write_quorum_set(&self) -> Self::QuorumSet {
        let rules = self.parts.iter().map(|config| config.write.clone());
        Joint::from_rules(rules)
    }

    /// Returns `true` only when both cross-intersections return `Some(true)`. A disproven or
    /// unproven intersection requires a joint config. Nodes and roles do not affect this check.
    fn supports_direct_transition_to(&self, other: &Self) -> bool {
        let read = self.read_quorum_set();
        let write = self.write_quorum_set();
        let other_read = other.read_quorum_set();
        let other_write = other.write_quorum_set();

        let read_meets_write = read.intersects_with(&other_write);
        let write_meets_read = write.intersects_with(&other_read);
        read_meets_write == Some(true) && write_meets_read == Some(true)
    }

    /// The configs that count for the node are the last config and each earlier config where it
    /// is a voter. Its role joins its roles in these configs. Its metadata comes from the newest
    /// config that holds it, and its voter flags come from the rules of every config.
    fn get_node(&self, node_id: &Self::NodeId) -> Option<NodeRef<'_, Self::Node>> {
        let mut node = None;
        let mut role = NodeRole::Empty;
        let mut is_active = false;

        // Newest config first, so the first config that holds the node gives its metadata.
        for (i, config) in self.parts.iter().rev().enumerate() {
            let Some((config_node, set_role)) = config.nodes.get(node_id) else {
                continue;
            };
            if node.is_none() {
                node = Some(config_node);
            }

            // The last config decides every node it holds. An earlier config still applies its
            // quorums, so it keeps the roles of its voters, but not of its learners.
            let is_voter = config.is_voter(node_id);
            let is_last = i == 0;
            if is_last || is_voter {
                role |= set_role.unwrap_or(default_role(is_voter));
                is_active = true;
            }
        }
        if !is_active {
            return None;
        }
        // Safe unwrap(): a config that counts for the node holds it.
        let node = node.unwrap();

        let is_read_voter = self.parts.iter().any(|config| has_id(&config.read, node_id));
        let is_write_voter = self.parts.iter().any(|config| has_id(&config.write, node_id));
        let node_ref = NodeRef {
            node,
            role,
            is_read_voter,
            is_write_voter,
        };
        Some(node_ref)
    }

    type Iter = std::vec::IntoIter<Q::Id>;

    /// Returns every active node ID once, in ascending order: the nodes of the last config and the
    /// voters that the earlier configs hold.
    fn node_ids(&self) -> Self::Iter {
        let mut ids = BTreeSet::new();
        if let Some((last, earlier)) = self.parts.split_last() {
            ids.extend(last.nodes.keys().cloned());
            for config in earlier {
                for node_id in config.voters() {
                    if config.nodes.contains_key(&node_id) {
                        ids.insert(node_id);
                    }
                }
            }
        }
        let ids: Vec<Q::Id> = ids.into_iter().collect();
        ids.into_iter()
    }

    /// Returns the configs of `self` followed by the configs of `goal`, as [`Joint::joint_with()`]
    /// does.
    fn joint_with(&self, goal: &Self) -> Self {
        Joint::joint_with(self, goal)
    }
}
