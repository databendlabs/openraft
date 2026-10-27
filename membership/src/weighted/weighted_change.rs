use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

use display_more::DisplayDebugOptionExt;
use display_more::DisplayOptionExt;
use openraft::Node;
use openraft::NodeId;
use openraft::errors::EmptyMembership;
use openraft::errors::MembershipError;
use openraft::membership::NodeRole;
use openraft::membership::RaftMembershipEdit;
use openraft_macros::since;

use crate::WeightedMembership;
use crate::config::Config;
use crate::errors::InvalidWeights;

/// A change to a [`WeightedMembership`]. [`apply()`](RaftMembershipEdit::apply) adds
/// `add_nodes`, sets the weights and the write weight, removes `remove_nodes`, then sets `roles`.
/// It derives the read weight from the new total weight.
///
/// The voters of the result are the IDs of its weights. A change keeps the weights and the write
/// weight unless it sets them, and the roles that the last config sets for the nodes that remain.
/// When new weights sum to less than the write weight, the change must also set a smaller write
/// weight. A weight of 0, weights that sum to more than `u64::MAX`, or a write weight outside
/// `1..=total_weight` make `apply()` return [`MembershipError::UserDefined`]. A voter without a
/// node makes `apply()` return [`MembershipError::NodeNotFound`].
///
/// A change of the weights or of the write weight goes through a joint config, as
/// [`WeightedMembership`] explains.
///
/// The result gives each voter [`NodeRole::Campaigner`] `|` [`NodeRole::ReceiveLogs`] and every
/// other node [`NodeRole::ReceiveLogs`], unless it sets another role. A node of `roles` that the
/// result lacks makes `apply()` return [`MembershipError::NodeNotFound`], and a write voter without
/// [`NodeRole::ReceiveLogs`] makes it return [`MembershipError::WriteVoterWithoutReceiveLogs`]. The
/// result keeps a role of the last config only while it differs from the default for the node's
/// status there: a voter with only [`NodeRole::ReceiveLogs`] that becomes a learner keeps that
/// role, the default for a learner, and it campaigns again once a later change makes it a voter.
/// When OpenRaft moves to the result through a joint config, the joint gives each node its role in
/// the result, joined with its old role if it is an old voter, and
/// [`make_uniform()`](RaftMembershipEdit::make_uniform) restores the roles of the result.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
/// use std::collections::BTreeSet;
///
/// use openraft::membership::RaftMembershipEdit;
/// use openraft_membership::{WeightedChange, WeightedMembership};
///
/// let weights = BTreeMap::from([(1, 1), (2, 1), (3, 1), (4, 1)]);
/// let nodes = BTreeSet::from([1, 2, 3, 4]);
/// let membership = WeightedMembership::<u64, ()>::new(weights, 3, nodes).unwrap();
///
/// // Voter 1 weighs 3, so the total weight becomes 6. The write weight stays 3, and the read
/// // weight becomes 4.
/// let change = WeightedChange::weights(BTreeMap::from([(1, 3), (2, 1), (3, 1), (4, 1)]));
/// let heavy = change.apply(&membership, true).unwrap();
/// let want = "{configs: [Config { read: WeightedQuorumSet { weights: {1: 3, 2: 1, 3: 1, 4: 1}, threshold: 4 }, \
///             write: WeightedQuorumSet { weights: {1: 3, 2: 1, 3: 1, 4: 1}, threshold: 3 }, \
///             nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
///             3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
/// assert_eq!(want, heavy.to_string());
///
/// // Removing voter 1 leaves weight 3. Write weight 2 keeps a majority; node 1 stays as a learner.
/// let change = WeightedChange::weights(BTreeMap::from([(2, 1), (3, 1), (4, 1)])).with_write_weight(2);
/// let shrunk = change.apply(&heavy, true).unwrap();
/// let want = "{configs: [Config { read: WeightedQuorumSet { weights: {2: 1, 3: 1, 4: 1}, threshold: 2 }, \
///             write: WeightedQuorumSet { weights: {2: 1, 3: 1, 4: 1}, threshold: 2 }, \
///             nodes: {1: ((), NodeRole(ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
///             3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
/// assert_eq!(want, shrunk.to_string());
/// ```
///
/// [`MembershipError::UserDefined`]: openraft::errors::MembershipError::UserDefined
/// [`MembershipError::NodeNotFound`]: openraft::errors::MembershipError::NodeNotFound
/// [`MembershipError::WriteVoterWithoutReceiveLogs`]: openraft::errors::MembershipError::WriteVoterWithoutReceiveLogs
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeightedChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Nodes to add as learners. A node that already exists keeps its metadata.
    pub add_nodes: BTreeMap<NID, N>,

    /// The new voters with their weights; `None` keeps the weights.
    pub weights: Option<BTreeMap<NID, u64>>,

    /// The new write weight; `None` keeps the current one.
    pub write_weight: Option<u64>,

    /// Nodes to remove. Removing a voter of the result fails.
    pub remove_nodes: BTreeSet<NID>,

    /// The roles to set in the result, by node ID; each node must be a node of the result. A node
    /// that it does not name keeps the role that the last config sets.
    pub roles: BTreeMap<NID, NodeRole>,
}

impl<NID, N> WeightedChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns a change of the voters and their weights that keeps the write weight and the nodes.
    pub fn weights(weights: BTreeMap<NID, u64>) -> Self {
        Self {
            weights: Some(weights),
            ..Self::default()
        }
    }

    /// Returns a change of the write weight that keeps the weights and the nodes.
    pub fn write_weight(write_weight: u64) -> Self {
        Self {
            write_weight: Some(write_weight),
            ..Self::default()
        }
    }

    /// Returns this change with the write weight set.
    pub fn with_write_weight(self, write_weight: u64) -> Self {
        Self {
            write_weight: Some(write_weight),
            ..self
        }
    }

    /// Returns this change with `roles` as the roles that it sets.
    pub fn with_roles(self, roles: BTreeMap<NID, NodeRole>) -> Self {
        Self { roles, ..self }
    }
}

impl<NID, N> Default for WeightedChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns the empty change, which keeps the weights, the write weight and the nodes.
    fn default() -> Self {
        Self {
            add_nodes: BTreeMap::new(),
            weights: None,
            write_weight: None,
            remove_nodes: BTreeSet::new(),
            roles: BTreeMap::new(),
        }
    }
}

impl<NID, N> fmt::Display for WeightedChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{add_nodes: {:?}, weights: {}, write_weight: {}, remove_nodes: {:?}, roles: {:?}}}",
            self.add_nodes,
            self.weights.display_debug(),
            self.write_weight.display(),
            self.remove_nodes,
            self.roles
        )
    }
}

/// `add_learner()` adds one node and keeps the weights and the write weight.
/// `make_uniform()` returns the empty change, which reduces a joint membership to its last config.
impl<NID, N> RaftMembershipEdit<WeightedMembership<NID, N>> for WeightedChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type Error = InvalidWeights<NID>;

    fn apply(
        self,
        membership: &WeightedMembership<NID, N>,
        retain: bool,
    ) -> Result<WeightedMembership<NID, N>, MembershipError<NID, Self::Error>> {
        tracing::debug!("WeightedChange::apply: change: {}", self);

        let Some(last) = membership.configs.last() else {
            return Err(EmptyMembership {}.into());
        };

        let last_rule = last.write();
        let weights = match self.weights {
            None => last_rule.weights().clone(),
            Some(weights) => weights,
        };
        let write_weight = self.write_weight.unwrap_or(last_rule.threshold());
        let (read, write) = WeightedMembership::<NID, N>::rules_from_weights(weights, write_weight)?;

        let nodes = Config::edit_nodes(&membership.configs, self.add_nodes, &self.remove_nodes);
        let roles = last.edit_roles(self.roles, &nodes)?;
        let config = Config::new(read, write, roles, nodes)?;

        let mut target = WeightedMembership { configs: vec![config] };
        if !retain {
            target.configs = Config::drop_voters_demoted_to_learners(&membership.configs, target.configs);
        }
        tracing::debug!("target membership: {}", target);
        Ok(target)
    }

    fn add_learner(node_id: NID, node: N) -> Self {
        let add_nodes = BTreeMap::from([(node_id, node)]);
        Self {
            add_nodes,
            ..Self::default()
        }
    }

    fn make_uniform() -> Self {
        Self::default()
    }
}
