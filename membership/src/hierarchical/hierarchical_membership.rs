use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::Node;
use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft::membership::IntoNodes;
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
/// [`with_roles()`](Self::with_roles) and [`HierarchicalChange`](crate::HierarchicalChange) set
/// node roles. The [shared role guide](crate::docs) explains defaults, leadership transfers and
/// joint changes.
///
/// The tree alone decides which nodes are voters. Both rules of a config share its tree, so every
/// voter is a read voter and a write voter. Every voter therefore needs
/// [`NodeRole::ReceiveLogs`], or a write quorum that holds the voter never commits.
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
///
/// [`NodeRole::ReceiveLogs`]: openraft::membership::NodeRole::ReceiveLogs
#[since(
    version = "0.10.0",
    change = "became an alias of `Joint<Config<N, HierarchicalQuorumSet<NID>>>`"
)]
#[since(version = "0.10.0")]
pub type HierarchicalMembership<NID, N> = Joint<Config<N, HierarchicalQuorumSet<NID>>>;

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

        let membership = Self::new(groups, nodes);
        membership.expect("a nonempty majority group and complete node metadata form a valid config")
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
        let config = Config::new(read, write, nodes)?;
        Ok(Self { parts: vec![config] })
    }
}
