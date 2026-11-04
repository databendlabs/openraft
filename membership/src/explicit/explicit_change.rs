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

use crate::ExplicitMembership;
use crate::NodeChange;
use crate::errors::InvalidQuorumLists;
use crate::quorum::ExplicitQuorumSet;

/// A change to an [`ExplicitMembership`]. [`apply()`](RaftMembershipEdit::apply) adds
/// `node_change.add_nodes`, sets the quorum lists, removes `node_change.remove_nodes`, then sets
/// `node_change.roles`.
///
/// A change keeps each quorum list that it does not set, and the roles that the last config sets
/// for the nodes that remain. A list that holds no quorum or an empty quorum, or a read quorum
/// that misses a write quorum, makes `apply()` return [`MembershipError::UserDefined`]. A voter
/// without a node makes `apply()` return [`MembershipError::NodeNotFound`].
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
/// its old role if it is an old voter, and [`make_uniform()`](RaftMembershipEdit::make_uniform)
/// restores the roles of the result.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
///
/// use openraft::membership::RaftMembershipEdit;
/// use openraft_membership::{ExplicitChange, ExplicitMembership};
///
/// let all = BTreeSet::from([1, 2, 3]);
/// let membership = ExplicitMembership::<u64, ()>::new([all.clone()], [all.clone()], all).unwrap();
///
/// // Any voter elects a leader; a commit still needs every voter.
/// let read = vec![BTreeSet::from([1]), BTreeSet::from([2]), BTreeSet::from([3])];
/// let change = ExplicitChange::read_quorums(read);
/// let changed = change.apply(&membership, true).unwrap();
/// let want = "{configs: [Config { read: ExplicitQuorumSet { quorums: [{1}, {2}, {3}] }, write: \
///             ExplicitQuorumSet { quorums: [{1, 2, 3}] }, nodes: {1: ((), None), 2: ((), None), \
///             3: ((), None)} }]}";
/// assert_eq!(want, changed.to_string());
/// ```
///
/// [`MembershipError::UserDefined`]: openraft::errors::MembershipError::UserDefined
/// [`MembershipError::NodeNotFound`]: openraft::errors::MembershipError::NodeNotFound
/// [`MembershipError::WriteVoterWithoutReceiveLogs`]: openraft::errors::MembershipError::WriteVoterWithoutReceiveLogs
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplicitChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// The node additions, removals and role assignments to apply.
    pub node_change: NodeChange<NID, N>,

    /// The new minimal read quorums; `None` keeps the read list.
    pub read: Option<Vec<BTreeSet<NID>>>,

    /// The new minimal write quorums; `None` keeps the write list.
    pub write: Option<Vec<BTreeSet<NID>>>,
}

impl<NID, N> ExplicitChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns a change of both quorum lists that keeps the nodes.
    pub fn quorums(read: Vec<BTreeSet<NID>>, write: Vec<BTreeSet<NID>>) -> Self {
        Self {
            read: Some(read),
            write: Some(write),
            ..Self::default()
        }
    }

    /// Returns a change of the read quorums that keeps the write quorums and the nodes.
    pub fn read_quorums(read: Vec<BTreeSet<NID>>) -> Self {
        Self {
            read: Some(read),
            ..Self::default()
        }
    }

    /// Returns a change of the write quorums that keeps the read quorums and the nodes.
    pub fn write_quorums(write: Vec<BTreeSet<NID>>) -> Self {
        Self {
            write: Some(write),
            ..Self::default()
        }
    }

    /// Returns this change with `roles` as the roles that it sets.
    pub fn with_roles(mut self, roles: BTreeMap<NID, NodeRole>) -> Self {
        self.node_change = self.node_change.with_roles(roles);
        self
    }
}

impl<NID, N> Default for ExplicitChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns the empty change, which keeps the quorum lists and the nodes.
    fn default() -> Self {
        Self {
            node_change: NodeChange::default(),
            read: None,
            write: None,
        }
    }
}

impl<NID, N> fmt::Display for ExplicitChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{add_nodes: {:?}, read: {}, write: {}, remove_nodes: {:?}, roles: {:?}}}",
            self.node_change.add_nodes,
            self.read.display_debug(),
            self.write.display_debug(),
            self.node_change.remove_nodes,
            self.node_change.roles
        )
    }
}

/// `add_learner()` adds one node and keeps the quorum lists.
/// `make_uniform()` returns the empty change, which reduces a joint membership to the rules of its
/// last config. `retain` decides whether the old voters that leave the voters stay as learners.
impl<NID, N> RaftMembershipEdit<ExplicitMembership<NID, N>> for ExplicitChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type Error = InvalidQuorumLists<NID>;

    fn apply(
        self,
        membership: &ExplicitMembership<NID, N>,
        retain: bool,
    ) -> Result<ExplicitMembership<NID, N>, MembershipError<NID, Self::Error>> {
        tracing::debug!("ExplicitChange::apply: change: {}", self);

        let Some(last) = membership.parts.last() else {
            return Err(EmptyMembership {}.into());
        };

        let read = match self.read {
            None => last.read().clone(),
            Some(read) => ExplicitQuorumSet::new(read).ok_or(InvalidQuorumLists::EmptyReadQuorum)?,
        };
        let write = match self.write {
            None => last.write().clone(),
            Some(write) => ExplicitQuorumSet::new(write).ok_or(InvalidQuorumLists::EmptyWriteQuorum)?,
        };
        ExplicitMembership::<NID, N>::check_rules(&read, &write)?;

        let target = membership.replace_config((read, write), self.node_change, retain)?;
        tracing::debug!("target membership: {}", target);
        Ok(target)
    }

    fn add_learner(node_id: NID, node: N) -> Self {
        let node_change = NodeChange::add_node(node_id, node);
        Self {
            node_change,
            ..Self::default()
        }
    }

    fn make_uniform() -> Self {
        Self::default()
    }
}
