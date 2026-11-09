use std::collections::BTreeMap;
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
use crate::NodeChange;
use crate::errors::InvalidGroups;
use crate::quorum::Group;
use crate::quorum::HierarchicalQuorumSet;

/// A change to a [`HierarchicalMembership`]. [`apply()`](RaftMembershipEdit::apply) adds
/// `node_change.add_nodes`, sets the groups with their write sizes, removes
/// `node_change.remove_nodes`, then sets `node_change.roles`. It derives the read sizes from the
/// result.
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
/// keeps the trees of both configs. OpenRaft then appends the computed target with its tree.
///
/// A node of the result takes the role that `node_change.roles` or the last config sets for it, and
/// otherwise the default for its status in the result: [`NodeRole::Campaigner`] `|`
/// [`NodeRole::ReceiveLogs`] for a voter, and [`NodeRole::ReceiveLogs`] for every other node. A set
/// role stays until a change sets another one: a voter with only [`NodeRole::ReceiveLogs`] that
/// becomes a learner, and later a voter again, still does not campaign. A role of
/// `node_change.roles` equal to the default for the node's status clears the set role. A node of
/// `node_change.roles` that the result lacks makes `apply()` return
/// [`MembershipError::NodeNotFound`]. The result keeps a role as given, even one without
/// [`NodeRole::ReceiveLogs`] for a write voter: OpenRaft rejects such a result with
/// [`MembershipError::WriteVoterWithoutReceiveLogs`] when it is proposed. When OpenRaft moves to
/// the result through a joint config, the joint gives each node its role in the result, joined with
/// its old role if it is an old voter. Appending the target restores the roles of the result.
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
/// let want = "{configs: [Config { read: HierarchicalQuorumSet { root: Group { quorum_size: 2, \
///             voters: {}, groups: {\"a\": Group { quorum_size: 1, voters: {1, 2}, groups: {} }, \
///             \"b\": Group { quorum_size: 2, voters: {3, 4, 5}, groups: {} }} } }, write: \
///             HierarchicalQuorumSet { root: Group { quorum_size: 1, voters: {}, groups: {\"a\": \
///             Group { quorum_size: 2, voters: {1, 2}, groups: {} }, \"b\": Group { quorum_size: \
///             2, voters: {3, 4, 5}, groups: {} }} } }, nodes: {1: ((), None), 2: ((), None), 3: \
///             ((), None), 4: ((), None), 5: ((), None)} }]}";
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
    /// The node additions, removals and role assignments to apply.
    pub node_change: NodeChange<NID, N>,

    /// The new root group, whose tree holds every voter, with the write size of each group as its
    /// `quorum_size`; `None` keeps the groups and their write sizes.
    pub groups: Option<Group<NID>>,
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
    pub fn with_roles(mut self, roles: BTreeMap<NID, NodeRole>) -> Self {
        self.node_change = self.node_change.with_roles(roles);
        self
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
            node_change: NodeChange::default(),
            groups: None,
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
            self.node_change.add_nodes,
            self.groups.display_debug(),
            self.node_change.remove_nodes,
            self.node_change.roles
        )
    }
}

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

        let Some(last) = membership.parts.last() else {
            return Err(EmptyMembership {}.into());
        };

        let write = match self.groups {
            None => last.write().clone(),
            Some(groups) => HierarchicalQuorumSet::new(groups)?,
        };
        let read = write.dual();

        let target = membership.replace_config((read, write), self.node_change, retain)?;
        tracing::debug!("target membership: {}", target);
        Ok(target)
    }
}
