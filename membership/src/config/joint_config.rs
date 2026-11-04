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
    #[since(
        version = "0.10.0",
        change = "shared by all config joints; returns NodeNotFound directly"
    )]
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
    /// Keeps the last config's set roles, then applies new roles and the retention policy.
    pub(crate) fn replace_config(
        &self,
        rules: (Q, Q),
        change: NodeChange<Q::Id, N>,
        retain: bool,
    ) -> Result<Self, NodeNotFound<Q::Id>> {
        let mut nodes = BTreeMap::new();
        if let Some(last) = self.parts.last() {
            for (node_id, (node, _)) in &last.nodes {
                nodes.insert(node_id.clone(), node.clone());
            }
        }
        change.edit_nodes(&mut nodes, self);

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
