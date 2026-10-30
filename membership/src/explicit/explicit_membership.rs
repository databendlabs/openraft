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
use crate::errors::InvalidQuorumLists;
use crate::quorum::ExplicitQuorumSet;
use crate::quorum::Joint;

/// A membership whose quorums are listed: each config lists its minimal read quorums and its
/// minimal write quorums.
///
/// A set of voters is a read quorum when it holds every voter of a listed read quorum, and a
/// write quorum when it holds every voter of a listed write quorum. A candidate becomes leader
/// only with votes from a read quorum, and an entry commits once a write quorum stores it. Every
/// listed read quorum must share a voter with every listed write quorum, so every election
/// reaches a voter that stores each committed entry. Read quorums may be disjoint from each other,
/// and so may write quorums.
///
/// The lists need not name the same voters. A voter of only the read list votes in elections but
/// is never needed to commit an entry; a voter of only the write list stores entries but never
/// needs to vote.
///
/// Listing quorums suits a policy that no size rule expresses, such as a quorum that must hold one
/// voter of each data center. A policy with many minimal quorums makes each list long: a majority
/// of `n` voters has `C(n, n/2 + 1)` of them, 126 for 9 voters and 6435 for 15. Construction and
/// each membership change compare every listed read quorum with every listed write quorum, so they
/// cost `O(r * w * n)` for `r` read quorums and `w` write quorums of at most `n` voters.
///
/// A membership holds two or more configs only during a joint change. Then a quorum must be a
/// quorum of every config. Rule i of
/// [`read_quorum_set()`](openraft::membership::RaftMembership::read_quorum_set) and rule i of
/// [`write_quorum_set()`](openraft::membership::RaftMembership::write_quorum_set) belong to
/// config i.
///
/// [`explicit_membership`](crate::docs::explicit) explains how to choose the quorum lists and how
/// to change them.
///
/// # Node roles
///
/// [`with_roles()`](Self::with_roles) and [`ExplicitChange`](crate::ExplicitChange) set node roles.
/// The [shared role guide](crate::docs) explains defaults, leadership transfers and joint changes.
///
/// The lists alone decide whether a node is a read voter or a write voter. A voter of only the
/// read lists may lack [`NodeRole::ReceiveLogs`], because it never needs to store an entry. A
/// voter of the write lists needs it, or a write quorum that holds the voter never commits.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
///
/// use openraft::membership::{QuorumSet, RaftMembership};
/// use openraft_membership::ExplicitMembership;
///
/// // Writes need both voters of one site, {1,2} or {3,4}. Reads need a voter of each site.
/// let write = [BTreeSet::from([1, 2]), BTreeSet::from([3, 4])];
/// let read = [
///     BTreeSet::from([1, 3]),
///     BTreeSet::from([1, 4]),
///     BTreeSet::from([2, 3]),
///     BTreeSet::from([2, 4]),
/// ];
/// let nodes = BTreeSet::from([1, 2, 3, 4]);
/// let membership = ExplicitMembership::<u64, ()>::new(read, write, nodes).unwrap();
///
/// let write_accepts = membership.write_quorum_set().is_quorum([3, 4].iter());
/// assert!(write_accepts);
/// let read_accepts = membership.read_quorum_set().is_quorum([3, 4].iter());
/// assert!(!read_accepts);
/// ```
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct ExplicitMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) configs: Vec<Config<NID, N, ExplicitQuorumSet<NID>>>,
}

/// Written by hand: `derive(Default)` would require `NID: Default` and `N: Default`.
impl<NID, N> Default for ExplicitMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn default() -> Self {
        Self { configs: Vec::new() }
    }
}

/// Makes every node a voter, and lists the set of all voters as the only read quorum and the only
/// write quorum, so every election and every commit needs every voter.
///
/// A majority would need `C(n, n/2 + 1)` listed quorums for `n` voters; one quorum keeps the list
/// short for any number of nodes.
impl<NID, N> From<BTreeMap<NID, N>> for ExplicitMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn from(nodes: BTreeMap<NID, N>) -> Self {
        let voters: BTreeSet<NID> = nodes.keys().cloned().collect();
        if voters.is_empty() {
            return Self::default();
        }

        let membership = Self::new([voters.clone()], [voters], nodes);
        membership.expect("identical nonempty quorums and complete node metadata form a valid config")
    }
}

impl<NID, N> fmt::Display for ExplicitMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{configs: {:?}}}", self.configs)
    }
}

impl<NID, N> ExplicitMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Builds a membership with one config that lists the minimal read quorums `read` and the
    /// minimal write quorums `write`.
    ///
    /// Each list is canonicalized as [`ExplicitQuorumSet::new()`] describes. Every voter, an ID of
    /// a listed quorum, needs a node in `nodes`, and the other nodes in `nodes` are learners.
    /// `nodes` implements [`IntoNodes`], so it can be a `BTreeMap<NID, N>`, a `HashMap<NID, N>`,
    /// or a `BTreeSet<NID>` when `N` is `()` or [`EmptyNode`](openraft::EmptyNode).
    ///
    /// Returns [`MembershipError::UserDefined`] when a list holds no quorum or
    /// holds an empty quorum, or when a listed read quorum and a listed write quorum share no
    /// voter. Returns [`MembershipError::NodeNotFound`] for a voter with no node.
    pub fn new<R, W, T>(read: R, write: W, nodes: T) -> Result<Self, MembershipError<NID, InvalidQuorumLists<NID>>>
    where
        R: IntoIterator<Item = BTreeSet<NID>>,
        W: IntoIterator<Item = BTreeSet<NID>>,
        T: IntoNodes<NID, N>,
    {
        let (read, write) = Self::rules_from_lists(read, write)?;

        let nodes = nodes.into_nodes();
        let config = Config::new(read, write, BTreeMap::new(), nodes)?;
        Ok(Self { configs: vec![config] })
    }

    /// Returns this membership with each node of `roles` set to its role there in the last config,
    /// the only config of a membership from [`new()`](Self::new). The other nodes keep their roles.
    ///
    /// Returns [`MembershipError::NodeNotFound`] for a node that this membership does not
    /// know, and [`MembershipError::WriteVoterWithoutReceiveLogs`] for a write voter
    /// without [`NodeRole::ReceiveLogs`].
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
    pub fn with_roles(
        self,
        roles: BTreeMap<NID, NodeRole>,
    ) -> Result<Self, MembershipError<NID, InvalidQuorumLists<NID>>> {
        let configs = Config::with_roles(self.configs, roles)?;
        Ok(Self { configs })
    }

    /// Returns the read rule and the write rule that list `read` and `write`, checked like
    /// [`new()`](Self::new).
    pub(crate) fn rules_from_lists<R, W>(
        read: R,
        write: W,
    ) -> Result<(ExplicitQuorumSet<NID>, ExplicitQuorumSet<NID>), InvalidQuorumLists<NID>>
    where
        R: IntoIterator<Item = BTreeSet<NID>>,
        W: IntoIterator<Item = BTreeSet<NID>>,
    {
        let read = ExplicitQuorumSet::new(read).ok_or(InvalidQuorumLists::EmptyReadQuorum)?;
        let write = ExplicitQuorumSet::new(write).ok_or(InvalidQuorumLists::EmptyWriteQuorum)?;
        Self::check_rules(&read, &write)?;
        Ok((read, write))
    }

    /// Returns an error unless both rules list a quorum and every listed read quorum shares a
    /// voter with every listed write quorum.
    pub(crate) fn check_rules(
        read: &ExplicitQuorumSet<NID>,
        write: &ExplicitQuorumSet<NID>,
    ) -> Result<(), InvalidQuorumLists<NID>> {
        if read.quorums().is_empty() {
            return Err(InvalidQuorumLists::NoReadQuorum);
        }
        if write.quorums().is_empty() {
            return Err(InvalidQuorumLists::NoWriteQuorum);
        }

        let Some((read, write)) = read.disjoint_quorums(write) else {
            return Ok(());
        };
        let disjoint = InvalidQuorumLists::DisjointQuorums {
            read: read.clone(),
            write: write.clone(),
        };
        Err(disjoint)
    }
}

impl<NID, N> RaftMembership for ExplicitMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type NodeId = NID;
    type Node = N;
    type QuorumSet = Joint<ExplicitQuorumSet<NID>>;

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
