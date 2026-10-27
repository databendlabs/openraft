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

use crate::HierarchicalMembership;
use crate::config::Config;
use crate::errors::InvalidGroups;
use crate::hierarchical::hierarchical_membership::pair_groups;
use crate::hierarchical::hierarchical_membership::split_groups;
use crate::quorum::HierarchicalQuorumSet;

/// A change to a [`HierarchicalMembership`]. [`apply()`](RaftMembershipEdit::apply) adds
/// `add_nodes`, sets the groups and the write thresholds, removes `remove_nodes`, then sets
/// `roles`. It derives the read thresholds from the result.
///
/// A change keeps the groups and `write_groups` unless it sets them. A group that keeps its ID
/// keeps its write size unless `write_sizes` names it, and a group with a new ID must be named
/// there. Moving a voter to another group is a change of `groups`. The roles that the last config
/// sets stay for the nodes that remain.
///
/// The result must pass the checks of [`HierarchicalMembership::new()`]. For example, a group that
/// loses voters keeps its write size only while that size fits its new voter count. A failed check
/// makes `apply()` return [`MembershipError::UserDefined`]. A voter without a node makes `apply()`
/// return [`MembershipError::NodeNotFound`].
///
/// OpenRaft moves to a result with other groups or other write thresholds through a joint config,
/// which keeps the groups of both configs.
/// [`make_uniform()`](RaftMembershipEdit::make_uniform) then selects the groups of the result.
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
/// use openraft_membership::{HierarchicalChange, HierarchicalMembership};
///
/// let groups = BTreeMap::from([
///     ("a".to_string(), BTreeSet::from([1, 2, 3])),
///     ("b".to_string(), BTreeSet::from([4, 5])),
/// ]);
/// let write_sizes = BTreeMap::from([("a".to_string(), 2), ("b".to_string(), 2)]);
/// let nodes = BTreeSet::from([1, 2, 3, 4, 5]);
/// let membership = HierarchicalMembership::<u64, ()>::new(groups, 1, write_sizes, nodes).unwrap();
///
/// // Move voter 3 to group b. Both groups keep their write sizes.
/// let groups = BTreeMap::from([
///     ("a".to_string(), BTreeSet::from([1, 2])),
///     ("b".to_string(), BTreeSet::from([3, 4, 5])),
/// ]);
/// let change = HierarchicalChange::groups(groups);
/// let moved = change.apply(&membership).unwrap();
/// let want = "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2}, \
///             quorum_size: 1 }, \"b\": Group { voters: {3, 4, 5}, quorum_size: 2 }}, quorum_groups: 2 }, \
///             write: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2}, quorum_size: 2 }, \
///             \"b\": Group { voters: {3, 4, 5}, quorum_size: 2 }}, quorum_groups: 1 }, \
///             nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
///             3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
///             5: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
/// let display = moved.to_string();
/// assert_eq!(want, display);
/// ```
///
/// [`MembershipError::UserDefined`]: openraft::errors::MembershipError::UserDefined
/// [`MembershipError::NodeNotFound`]: openraft::errors::MembershipError::NodeNotFound
/// [`MembershipError::WriteVoterWithoutReceiveLogs`]: openraft::errors::MembershipError::WriteVoterWithoutReceiveLogs
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HierarchicalChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Nodes to add as learners. A node that already exists keeps its metadata.
    pub add_nodes: BTreeMap<NID, N>,

    /// The new groups, the voters of each group by group ID; `None` keeps the groups.
    pub groups: Option<BTreeMap<String, BTreeSet<NID>>>,

    /// The new number of groups that a write quorum must reach; `None` keeps the current one.
    pub write_groups: Option<u64>,

    /// The new write sizes of the groups that it names. A group that it does not name keeps its
    /// write size.
    pub write_sizes: BTreeMap<String, u64>,

    /// Nodes to remove. Removing a voter of the result fails.
    pub remove_nodes: BTreeSet<NID>,

    /// The roles to set in the result, by node ID; each node must be a node of the result. A node
    /// that it does not name keeps the role that the last config sets.
    pub roles: BTreeMap<NID, NodeRole>,
}

impl<NID, N> HierarchicalChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns a change of the groups that keeps `write_groups`, the write sizes of the groups
    /// that keep their IDs, and the nodes.
    pub fn groups(groups: BTreeMap<String, BTreeSet<NID>>) -> Self {
        Self {
            groups: Some(groups),
            ..Self::default()
        }
    }

    /// Returns a change of `write_groups` that keeps the groups, the write sizes and the nodes.
    pub fn write_groups(write_groups: u64) -> Self {
        Self {
            write_groups: Some(write_groups),
            ..Self::default()
        }
    }

    /// Returns a change of the write sizes of the groups that `write_sizes` names, which keeps the
    /// groups, `write_groups`, the other write sizes and the nodes.
    pub fn write_sizes(write_sizes: BTreeMap<String, u64>) -> Self {
        Self {
            write_sizes,
            ..Self::default()
        }
    }

    /// Returns this change with `write_groups` set.
    pub fn with_write_groups(self, write_groups: u64) -> Self {
        Self {
            write_groups: Some(write_groups),
            ..self
        }
    }

    /// Returns this change with `write_sizes` as the write sizes that it sets.
    pub fn with_write_sizes(self, write_sizes: BTreeMap<String, u64>) -> Self {
        Self { write_sizes, ..self }
    }

    /// Returns this change with `roles` as the roles that it sets.
    pub fn with_roles(self, roles: BTreeMap<NID, NodeRole>) -> Self {
        Self { roles, ..self }
    }
}

impl<NID, N> Default for HierarchicalChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns the empty change, which keeps the groups, the write thresholds and the nodes.
    fn default() -> Self {
        Self {
            add_nodes: BTreeMap::new(),
            groups: None,
            write_groups: None,
            write_sizes: BTreeMap::new(),
            remove_nodes: BTreeSet::new(),
            roles: BTreeMap::new(),
        }
    }
}

impl<NID, N> fmt::Display for HierarchicalChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{add_nodes: {:?}, groups: {}, write_groups: {}, write_sizes: {:?}, remove_nodes: {:?}, roles: {:?}}}",
            self.add_nodes,
            self.groups.display_debug(),
            self.write_groups.display(),
            self.write_sizes,
            self.remove_nodes,
            self.roles
        )
    }
}

/// `add_learner()` adds one node and keeps the groups and the write thresholds.
/// `make_uniform()` returns the empty change, which reduces a joint membership to its last config.
impl<NID, N> RaftMembershipEdit<HierarchicalMembership<NID, N>> for HierarchicalChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type Error = InvalidGroups<NID>;

    fn apply(
        self,
        membership: &HierarchicalMembership<NID, N>,
    ) -> Result<HierarchicalMembership<NID, N>, MembershipError<NID, Self::Error>> {
        tracing::debug!("HierarchicalChange::apply: change: {}", self);

        let Some(last) = membership.configs.last() else {
            return Err(EmptyMembership {}.into());
        };
        let last_rule = last.write();
        let (last_groups, mut write_sizes) = split_groups(last_rule);

        let groups = match self.groups {
            None => last_groups,
            Some(groups) => groups,
        };
        let write_groups = self.write_groups.unwrap_or(last_rule.quorum_groups());

        write_sizes.retain(|group, _| groups.contains_key(group));
        write_sizes.extend(self.write_sizes);

        let groups = pair_groups(groups, write_sizes)?;
        let write = HierarchicalQuorumSet::new(write_groups, groups)?;
        let read = write.dual();

        let nodes = Config::edit_nodes(&membership.configs, self.add_nodes, &self.remove_nodes);
        let roles = last.edit_roles(self.roles, &nodes)?;
        let config = Config::new(read, write, roles, nodes)?;

        let target = HierarchicalMembership { configs: vec![config] };
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
