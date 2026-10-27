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
use crate::config::Config;
use crate::errors::InvalidQuorumLists;
use crate::quorum::ExplicitQuorumSet;

/// A change to an [`ExplicitMembership`]. [`apply()`](RaftMembershipEdit::apply) adds
/// `add_nodes`, sets the quorum lists, removes `remove_nodes`, then sets `roles`.
///
/// A change keeps each quorum list that it does not set, and the roles that the last config sets
/// for the nodes that remain. A list that holds no quorum or an empty quorum, or a read quorum
/// that misses a write quorum, makes `apply()` return [`MembershipError::UserDefined`]. A voter
/// without a node makes `apply()` return [`MembershipError::NodeNotFound`].
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
/// use openraft_membership::{ExplicitChange, ExplicitMembership};
///
/// let all = BTreeSet::from([1, 2, 3]);
/// let membership = ExplicitMembership::<u64, ()>::new([all.clone()], [all.clone()], all).unwrap();
///
/// // Any voter elects a leader; a commit still needs every voter.
/// let read = vec![BTreeSet::from([1]), BTreeSet::from([2]), BTreeSet::from([3])];
/// let change = ExplicitChange::read_quorums(read);
/// let changed = change.apply(&membership).unwrap();
/// let want = "{configs: [Config { read: ExplicitQuorumSet { quorums: [{1}, {2}, {3}] }, write: ExplicitQuorumSet { \
///             quorums: [{1, 2, 3}] }, nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
///             2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
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
    /// Nodes to add as learners. A node that already exists keeps its metadata.
    pub add_nodes: BTreeMap<NID, N>,

    /// The new minimal read quorums; `None` keeps the read list.
    pub read: Option<Vec<BTreeSet<NID>>>,

    /// The new minimal write quorums; `None` keeps the write list.
    pub write: Option<Vec<BTreeSet<NID>>>,

    /// Nodes to remove. Removing a voter of the result fails.
    pub remove_nodes: BTreeSet<NID>,

    /// The roles to set in the result, by node ID; each node must be a node of the result. A node
    /// that it does not name keeps the role that the last config sets.
    pub roles: BTreeMap<NID, NodeRole>,
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
    pub fn with_roles(self, roles: BTreeMap<NID, NodeRole>) -> Self {
        Self { roles, ..self }
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
            add_nodes: BTreeMap::new(),
            read: None,
            write: None,
            remove_nodes: BTreeSet::new(),
            roles: BTreeMap::new(),
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
            self.add_nodes,
            self.read.display_debug(),
            self.write.display_debug(),
            self.remove_nodes,
            self.roles
        )
    }
}

/// `add_learner()` adds one node and keeps the quorum lists.
/// `make_uniform()` returns the empty change, which reduces a joint membership to its last config.
impl<NID, N> RaftMembershipEdit<ExplicitMembership<NID, N>> for ExplicitChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type Error = InvalidQuorumLists<NID>;

    fn apply(
        self,
        membership: &ExplicitMembership<NID, N>,
    ) -> Result<ExplicitMembership<NID, N>, MembershipError<NID, Self::Error>> {
        tracing::debug!("ExplicitChange::apply: change: {}", self);

        let Some(last) = membership.configs.last() else {
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

        let nodes = Config::edit_nodes(&membership.configs, self.add_nodes, &self.remove_nodes);
        let roles = last.edit_roles(self.roles, &nodes)?;
        let config = Config::new(read, write, roles, nodes)?;

        let target = ExplicitMembership { configs: vec![config] };
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
