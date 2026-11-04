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
use openraft_macros::since;

use crate::config::Config;
use crate::config::default_role;
use crate::config::has_id;
use crate::node_change::NodeChange;
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
    /// Returns the configs of this membership, oldest first. It holds more than one config only
    /// during a joint change, and then the last one is the target of the change.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use std::collections::BTreeSet;
    ///
    /// use openraft::membership::NodeRole;
    /// use openraft_membership::WeightedMembership;
    ///
    /// let weights = BTreeMap::from([(1, 2), (2, 1), (3, 1)]);
    /// let nodes = BTreeSet::from([1, 2, 3]);
    /// let membership = WeightedMembership::<u64, ()>::new(weights.clone(), 3, nodes).unwrap();
    /// let roles = BTreeMap::from([(3, NodeRole::ReceiveLogs)]);
    /// let membership = membership.with_roles(roles).unwrap();
    ///
    /// // A membership outside a joint change holds one config.
    /// let configs = membership.configs();
    /// assert_eq!(1, configs.len());
    /// let config = &configs[0];
    ///
    /// // Its write rule holds the weights and the write weight.
    /// assert_eq!(&weights, config.write().weights());
    /// assert_eq!(3, config.write().threshold());
    ///
    /// // Voter 3 has a set role, and voter 1 follows the default for a voter.
    /// let (_, role_of_3) = &config.nodes()[&3];
    /// assert_eq!(&Some(NodeRole::ReceiveLogs), role_of_3);
    /// let (_, role_of_1) = &config.nodes()[&1];
    /// assert_eq!(&None, role_of_1);
    /// ```
    #[since(version = "0.10.0")]
    pub fn configs(&self) -> &[Config<N, Q>] {
        &self.parts
    }
    /// Returns this membership with each node of `roles` set to its role there in the last config.
    /// The other nodes keep their roles. Setting a node to its default role clears its stored role,
    /// so it follows its default again.
    ///
    /// Returns [`NodeNotFound`] for the first node of `roles` that the last config does not hold.
    /// A role is kept as given, even one without [`NodeRole::ReceiveLogs`] for a write voter:
    /// OpenRaft rejects such a membership with [`WriteVoterWithoutReceiveLogs`] when it is
    /// initialized or proposed.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use std::collections::BTreeSet;
    ///
    /// use openraft::membership::{NodeRef, NodeRole, RaftMembership};
    /// use openraft_membership::ExplicitMembership;
    ///
    /// // Any two of voters 1, 2 and 3 form a quorum.
    /// let majorities = [BTreeSet::from([1, 2]), BTreeSet::from([1, 3]), BTreeSet::from([2, 3])];
    /// let nodes = BTreeSet::from([1, 2, 3]);
    /// let membership = ExplicitMembership::<u64, ()>::new(majorities.clone(), majorities, nodes).unwrap();
    ///
    /// // Voter 3 stores logs and votes, but never starts an election on its own.
    /// let roles = BTreeMap::from([(3, NodeRole::ReceiveLogs)]);
    /// let membership = membership.with_roles(roles).unwrap();
    ///
    /// let got = membership.get_node(&3);
    /// let want = NodeRef {
    ///     node: &(),
    ///     role: NodeRole::ReceiveLogs,
    ///     is_read_voter: true,
    ///     is_write_voter: true,
    /// };
    /// assert_eq!(Some(want), got);
    /// ```
    ///
    /// [`WriteVoterWithoutReceiveLogs`]: openraft::errors::MembershipError::WriteVoterWithoutReceiveLogs
    #[since(version = "0.10.0")]
    pub fn with_roles(mut self, roles: BTreeMap<Q::Id, NodeRole>) -> Result<Self, NodeNotFound<Q::Id>> {
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

    /// Returns a membership with `rules` as its only config, applying the node edits in `change`.
    ///
    /// The edits start from every active node, with the metadata that `get_node()` gives it: the
    /// nodes of the last config and the voters of the earlier configs. An earlier voter that the
    /// last config lacks still counts in the quorums of this membership, so new rules may select
    /// it, and `retain` decides whether it stays as a learner. Such a voter has no set role in the
    /// result: the change that built the last config removed it. The result keeps the set roles of
    /// the last config, then applies new roles and the retention policy.
    pub(crate) fn replace_config(
        &self,
        rules: (Q, Q),
        change: NodeChange<Q::Id, N>,
        retain: bool,
    ) -> Result<Self, NodeNotFound<Q::Id>> {
        let mut nodes = BTreeMap::new();
        for node_id in self.node_ids() {
            // Safe unwrap(): `node_ids()` returns only active nodes, and `get_node()` finds each.
            let node = self.get_node(&node_id).unwrap();
            nodes.insert(node_id, node.node.clone());
        }
        change.edit_nodes(&mut nodes);

        let (read, write) = rules;
        let mut config = Config::new(read, write, nodes)?;
        if let Some(last) = self.parts.last() {
            config.keep_roles(last);
        }
        config.set_roles(change.roles)?;

        let mut target = Self { parts: vec![config] };
        if !retain {
            target = self.drop_voters_demoted_to_learners(target);
        }
        Ok(target)
    }

    /// Returns `goal` without the voters of this membership that are learners of `goal`, as
    /// [`NodeRef::is_learner()`] defines them. Such a node leaves every config of `goal`.
    pub(crate) fn drop_voters_demoted_to_learners(&self, mut goal: Self) -> Self {
        let mut demoted = BTreeSet::new();
        for config in &self.parts {
            demoted.extend(config.voters());
        }
        demoted.retain(|node_id| {
            let Some(node) = goal.get_node(node_id) else {
                return false;
            };
            node.is_learner()
        });

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
/// in each earlier config where it is a voter, and the metadata of the newest of these configs. A
/// joint therefore keeps every capability of the old voters, and selecting its last config
/// restores the roles of the target.
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
    /// is a voter. Its role joins its roles in these configs, and its metadata comes from the
    /// newest of them. An earlier config that holds the node only as a learner gives it nothing:
    /// the change to a later config removed that learner. Its voter flags come from the rules of
    /// every config.
    fn get_node(&self, node_id: &Self::NodeId) -> Option<NodeRef<'_, Self::Node>> {
        let mut node = None;
        let mut role = NodeRole::Empty;

        // Newest config first, so the first config that counts for the node gives its metadata.
        for (i, config) in self.parts.iter().rev().enumerate() {
            let Some((config_node, set_role)) = config.nodes.get(node_id) else {
                continue;
            };

            // The last config decides every node it holds. An earlier config still applies its
            // quorums, so it keeps the roles of its voters, but not of its learners.
            let is_voter = config.is_voter(node_id);
            let is_last = i == 0;
            if !is_last && !is_voter {
                continue;
            }

            if node.is_none() {
                node = Some(config_node);
            }
            role |= set_role.unwrap_or(default_role(is_voter));
        }
        // No config counts for the node, so it is not active.
        let node = node?;

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
    /// does, with one metadata for each active node in every config that holds it: the metadata
    /// that `goal` gives the node when `goal` knows it, and otherwise the metadata that `self`
    /// gives it.
    ///
    /// Without this step, the configs could disagree. A config of `goal` may hold an old voter as a
    /// learner that a later config of `goal` removed, and a config of `self` may hold metadata that
    /// `self` replaced in its last config. `get_node()` reads only the configs that count for a
    /// node, which would then pick discarded metadata.
    fn joint_with(&self, goal: &Self) -> Self {
        let mut joint = Joint::joint_with(self, goal);

        let mut metadata = BTreeMap::new();
        for node_id in joint.node_ids() {
            let known = match goal.get_node(&node_id) {
                Some(known) => known,
                // Safe unwrap(): a node of the joint that `goal` does not know is a voter of
                // `self`, and a voter is active.
                None => self.get_node(&node_id).unwrap(),
            };
            metadata.insert(node_id, known.node.clone());
        }

        for config in &mut joint.parts {
            for (node_id, (node, _)) in &mut config.nodes {
                if let Some(resolved) = metadata.get(node_id) {
                    *node = resolved.clone();
                }
            }
        }
        joint
    }
}
