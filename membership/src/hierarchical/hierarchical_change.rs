use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

use display_more::DisplayDebugOptionExt;
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
use crate::quorum::Group;
use crate::quorum::HierarchicalQuorumSet;

/// A change to a [`HierarchicalMembership`]. [`apply()`](RaftMembershipEdit::apply) adds
/// `add_nodes`, sets the groups with their write sizes, removes `remove_nodes`, then sets `roles`.
/// It derives the read sizes from the result.
///
/// A change keeps the groups and their write sizes unless it sets a new root group, and keeps the
/// roles that the last config sets for the nodes that remain. A new root group can move voters to
/// other groups, add or remove groups at any level, and change any write size.
///
/// The result must pass the checks of [`HierarchicalMembership::new()`]. A failed check makes
/// `apply()` return [`MembershipError::UserDefined`]. A voter without a node makes `apply()` return
/// [`MembershipError::NodeNotFound`].
///
/// OpenRaft moves to a result with other groups or other write sizes through a joint config, which
/// keeps the trees of both configs. [`make_uniform()`](RaftMembershipEdit::make_uniform) then
/// selects the tree of the result.
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
/// use std::collections::BTreeSet;
///
/// use openraft::membership::RaftMembershipEdit;
/// use openraft_membership::quorum::Group;
/// use openraft_membership::{HierarchicalChange, HierarchicalMembership};
///
/// let groups = Group::of_groups(1, [
///     ("a", Group::of_voters(2, [1, 2, 3])),
///     ("b", Group::of_voters(2, [4, 5])),
/// ]);
/// let nodes = BTreeSet::from([1, 2, 3, 4, 5]);
/// let membership = HierarchicalMembership::<u64, ()>::new(groups, nodes).unwrap();
///
/// // Move voter 3 to group b, where a write still needs 2 voters.
/// let groups = Group::of_groups(1, [
///     ("a", Group::of_voters(2, [1, 2])),
///     ("b", Group::of_voters(2, [3, 4, 5])),
/// ]);
/// let change = HierarchicalChange::groups(groups);
/// let moved = change.apply(&membership, true).unwrap();
/// let want = "{configs: [Config { read: HierarchicalQuorumSet { root: Group { quorum_size: 2, voters: {}, \
///             groups: {\"a\": Group { quorum_size: 1, voters: {1, 2}, groups: {} }, \"b\": Group { \
///             quorum_size: 2, voters: {3, 4, 5}, groups: {} }} } }, write: HierarchicalQuorumSet { \
///             root: Group { quorum_size: 1, voters: {}, groups: {\"a\": Group { quorum_size: 2, \
///             voters: {1, 2}, groups: {} }, \"b\": Group { quorum_size: 2, voters: {3, 4, 5}, groups: {} }} } }, \
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

    /// The new root group, whose tree holds every voter, with the write size of each group as its
    /// `quorum_size`; `None` keeps the groups and their write sizes.
    pub groups: Option<Group<NID>>,

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
    /// Returns a change to the root group `groups`, with the write sizes that it sets, which keeps
    /// the nodes.
    pub fn groups(groups: Group<NID>) -> Self {
        Self {
            groups: Some(groups),
            ..Self::default()
        }
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
    /// Returns the empty change, which keeps the groups, the write sizes and the nodes.
    fn default() -> Self {
        Self {
            add_nodes: BTreeMap::new(),
            groups: None,
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
            "{{add_nodes: {:?}, groups: {}, remove_nodes: {:?}, roles: {:?}}}",
            self.add_nodes,
            self.groups.display_debug(),
            self.remove_nodes,
            self.roles
        )
    }
}

/// `add_learner()` adds one node and keeps the groups and the write sizes.
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
        retain: bool,
    ) -> Result<HierarchicalMembership<NID, N>, MembershipError<NID, Self::Error>> {
        tracing::debug!("HierarchicalChange::apply: change: {}", self);

        let Some(last) = membership.configs.last() else {
            return Err(EmptyMembership {}.into());
        };

        let write = match self.groups {
            None => last.write().clone(),
            Some(groups) => HierarchicalQuorumSet::new(groups)?,
        };
        let read = write.dual();

        let nodes = Config::edit_nodes(&membership.configs, self.add_nodes, &self.remove_nodes);
        let roles = last.edit_roles(self.roles, &nodes)?;
        let config = Config::new(read, write, roles, nodes)?;

        let mut target = HierarchicalMembership { configs: vec![config] };
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
