use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::Node;
use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft::membership::IntoNodes;
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
///
/// [`NodeRole::ReceiveLogs`]: openraft::membership::NodeRole::ReceiveLogs
#[since(
    version = "0.10.0",
    change = "became an alias of `Joint<Config<N, ExplicitQuorumSet<NID>>>`"
)]
#[since(version = "0.10.0")]
pub type ExplicitMembership<NID, N> = Joint<Config<N, ExplicitQuorumSet<NID>>>;

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
        let config = Config::new(read, write, nodes)?;
        Ok(Self { parts: vec![config] })
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
