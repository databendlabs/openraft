use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

use openraft::Node;
use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft::membership::IntoNodes;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::RaftMembership;
use openraft_macros::since;

use crate::config::Config;
use crate::errors::InvalidGroups;
use crate::quorum::Group;
use crate::quorum::HierarchicalQuorumSet;
use crate::quorum::Joint;

/// A membership whose voters form a tree of groups, such as datacenters that hold racks: a quorum
/// needs enough members in enough groups, at every level.
///
/// A config arranges its voters in a tree of [`Group`]s. Each group holds voters, child groups, or
/// both: its members. Each group has an ID that is stable among its siblings, such as the name of
/// a zone, and each voter belongs to exactly one group. The config gives every group a
/// `write_size`. A set reaches a group when the voters of the group that it holds, plus the child
/// groups that it reaches, number at least the size of the group. A write quorum reaches the root
/// group with every group at its `write_size`. A read quorum reaches the root group with every
/// group at its `read_size = member_count - write_size + 1`. A candidate becomes leader only with
/// votes from a read quorum, and an entry commits once a write quorum stores it.
///
/// A read quorum and a write quorum reach a common member of the root group, because
/// `read_size + write_size` exceeds its member count. A common voter is a shared voter, and a
/// common child group repeats the argument one level down. So the two quorums share a voter. The
/// read quorums are exactly the sets that meet every write quorum, so an election needs no more
/// votes than safety requires.
///
/// Writes commit while the live voters reach the root group with the write sizes, and elections
/// succeed while they reach it with the read sizes. Raising the write size of a group lowers its
/// read size, so writes tolerate fewer failures there and elections more.
///
/// A membership holds two or more configs only during a joint change. Then a quorum must be a
/// quorum of every config. Rule i of
/// [`read_quorum_set()`](openraft::membership::RaftMembership::read_quorum_set) and rule i of
/// [`write_quorum_set()`](openraft::membership::RaftMembership::write_quorum_set) belong to
/// config i and share its tree.
///
/// [`hierarchical_membership`](crate::docs::hierarchical) explains how to arrange the groups and
/// choose the write sizes, and how to change them.
///
/// # Node roles
///
/// A [`NodeRole`] says whether a node may start an election and whether it receives logs. A config
/// gives each of its voters [`NodeRole::Campaigner`] `|` [`NodeRole::ReceiveLogs`] and every other
/// node [`NodeRole::ReceiveLogs`], unless it sets another role. [`with_roles()`](Self::with_roles)
/// sets roles in a membership, and the `roles` of a
/// [`HierarchicalChange`](crate::HierarchicalChange) set them in its result. A voter with only
/// [`NodeRole::ReceiveLogs`] stores logs and votes, but starts no election on its own; a
/// leadership transfer from the leader can still make it leader.
///
/// The tree alone decides which nodes are voters. Both rules of a config share its tree, so every
/// voter is a read voter and a write voter. Every voter therefore needs
/// [`NodeRole::ReceiveLogs`], or a write quorum that holds the voter never commits.
///
/// Each config holds every one of its nodes with its role there. A change keeps a role of the last
/// config only while it differs from the default for the node's status there. A voter with only
/// [`NodeRole::ReceiveLogs`] that a change turns into a learner therefore keeps that role, the
/// default for a learner, and it campaigns again once a later change makes it a voter. A joint
/// config gives each node its role in the new config, joined with its role in the old config if it
/// is a voter of the old config.
/// [`make_uniform()`](openraft::membership::RaftMembershipEdit::make_uniform) reduces the joint to
/// the new config and so restores the roles that the new config sets.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
///
/// use openraft::membership::{QuorumSet, RaftMembership};
/// use openraft_membership::HierarchicalMembership;
/// use openraft_membership::quorum::Group;
///
/// // Three zones of three voters. Reads and writes each need 2 voters in each of 2 zones.
/// let groups = Group::of_groups(2, [
///     ("a", Group::of_voters(2, [1, 2, 3])),
///     ("b", Group::of_voters(2, [4, 5, 6])),
///     ("c", Group::of_voters(2, [7, 8, 9])),
/// ]);
/// let nodes = BTreeSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9]);
/// let membership = HierarchicalMembership::<u64, ()>::new(groups, nodes).unwrap();
///
/// // Zone c is down, and zone a lost voter 1: writes still commit.
/// let write = membership.write_quorum_set();
/// let write_accepts = write.is_quorum([2, 3, 4, 5, 6].iter());
/// assert!(write_accepts);
/// // Zone c is down, and zone a lost two voters: only zone b keeps at least 2 voters.
/// let read = membership.read_quorum_set();
/// let read_accepts = read.is_quorum([3, 4, 5, 6].iter());
/// assert!(!read_accepts);
/// ```
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct HierarchicalMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) configs: Vec<Config<NID, N, HierarchicalQuorumSet<NID>>>,
}

/// Written by hand: `derive(Default)` would require `NID: Default` and `N: Default`.
impl<NID, N> Default for HierarchicalMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn default() -> Self {
        Self { configs: Vec::new() }
    }
}

/// Makes every node a voter of the root group, with `write_size = n / 2 + 1` for `n` voters. A read
/// quorum then holds `n - n / 2` voters.
impl<NID, N> From<BTreeMap<NID, N>> for HierarchicalMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn from(nodes: BTreeMap<NID, N>) -> Self {
        let voters: BTreeSet<NID> = nodes.keys().cloned().collect();
        if voters.is_empty() {
            return Self::default();
        }

        let voter_count = voters.len() as u64;
        let write_size = voter_count / 2 + 1;
        let groups = Group::of_voters(write_size, voters);

        // Safe unwrap(): the root group holds a voter, and 1 <= write_size <= voter_count.
        let write = HierarchicalQuorumSet::new(groups).unwrap();
        let read = write.dual();
        // Safe unwrap(): every voter is a node, and the config sets no role.
        let config = Config::new(read, write, BTreeMap::new(), nodes).unwrap();
        Self { configs: vec![config] }
    }
}

impl<NID, N> fmt::Display for HierarchicalMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{configs: {:?}}}", self.configs)
    }
}

impl<NID, N> HierarchicalMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Builds a membership with one config that arranges the voters in the tree of `groups`, the
    /// root group. The `quorum_size` of each group is its write size, and the read sizes are
    /// derived as the type docs describe.
    ///
    /// Every voter needs a node in `nodes`, and the other nodes in `nodes` are learners. `nodes`
    /// implements [`IntoNodes`], so it can be a `BTreeMap<NID, N>`, a `HashMap<NID, N>`, or a
    /// `BTreeSet<NID>` when `N` is `()` or [`EmptyNode`](openraft::EmptyNode).
    ///
    /// Returns [`MembershipError::UserDefined`] when `groups` fails the checks of
    /// [`HierarchicalQuorumSet::new()`]. Returns [`MembershipError::NodeNotFound`] for a voter with
    /// no node.
    pub fn new<T>(groups: Group<NID>, nodes: T) -> Result<Self, MembershipError<NID, InvalidGroups<NID>>>
    where T: IntoNodes<NID, N> {
        let write = HierarchicalQuorumSet::new(groups)?;
        let read = write.dual();

        let nodes = nodes.into_nodes();
        let config = Config::new(read, write, BTreeMap::new(), nodes)?;
        Ok(Self { configs: vec![config] })
    }

    /// Returns this membership with each node of `roles` set to its role there in the last config,
    /// the only config of a membership from [`new()`](Self::new). The other nodes keep their roles.
    ///
    /// Returns [`MembershipError::NodeNotFound`] for a node that this membership does
    /// not know, and [`MembershipError::WriteVoterWithoutReceiveLogs`] for a write
    /// voter without [`NodeRole::ReceiveLogs`].
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use std::collections::BTreeSet;
    ///
    /// use openraft::membership::{NodeRef, NodeRole, RaftMembership};
    /// use openraft_membership::HierarchicalMembership;
    /// use openraft_membership::quorum::Group;
    ///
    /// // One group of three voters; any two of them form a quorum.
    /// let groups = Group::of_voters(2, [1, 2, 3]);
    /// let nodes = BTreeSet::from([1, 2, 3]);
    /// let membership = HierarchicalMembership::<u64, ()>::new(groups, nodes).unwrap();
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
    pub fn with_roles(self, roles: BTreeMap<NID, NodeRole>) -> Result<Self, MembershipError<NID, InvalidGroups<NID>>> {
        let configs = Config::with_roles(self.configs, roles)?;
        Ok(Self { configs })
    }
}

impl<NID, N> RaftMembership for HierarchicalMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type NodeId = NID;
    type Node = N;
    type QuorumSet = Joint<HierarchicalQuorumSet<NID>>;

    fn read_quorum_set(&self) -> Self::QuorumSet {
        Config::read_quorum_set(&self.configs)
    }

    fn write_quorum_set(&self) -> Self::QuorumSet {
        Config::write_quorum_set(&self.configs)
    }

    fn supports_direct_transition_to(&self, other: &Self) -> bool {
        Config::supports_direct_transition(&self.configs, &other.configs)
    }

    fn get_node(&self, node_id: &Self::NodeId) -> Option<NodeRef<'_, Self::Node>> {
        Config::get_node(&self.configs, node_id)
    }

    type Iter = std::vec::IntoIter<NID>;

    fn node_ids(&self) -> Self::Iter {
        Config::node_ids(&self.configs)
    }

    fn joint_with(&self, goal: &Self) -> Self {
        let configs = Config::joint(&self.configs, &goal.configs);
        Self { configs }
    }
}
