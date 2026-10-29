use std::collections::BTreeMap;
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
use crate::errors::InvalidGrid;
use crate::quorum::GridQuorumSet;
use crate::quorum::GridRule;
use crate::quorum::Joint;

/// A membership whose voters form a grid: a read quorum holds every voter of some row, and a
/// write quorum holds at least one voter of every row.
///
/// Each config places its voters in a rectangular grid, one distinct voter per cell, given as its
/// rows. A candidate becomes leader only with votes from a complete row, and an entry commits
/// once a voter of each row stores it. These voters can sit in different columns. A write quorum
/// holds a voter of every row, so every election reaches a voter that stores each committed
/// entry. Two rows share no voter.
///
/// A grid of `r` rows and `c` columns holds `r * c` voters. A read quorum needs the `c` voters of
/// a row, and a write quorum `r` voters, one of each row. So in a square grid of 9 voters, reads
/// and writes each need 3 voters, while a majority needs 5. Elections tolerate fewer failures in
/// return: `r` failed voters, one in each row, stop every election. Commits continue while every
/// row retains a live voter, so only the failure of a whole row stops every commit.
///
/// A membership holds two or more configs only during a joint change. Then a quorum must be a
/// quorum of every config. Rule i of
/// [`read_quorum_set()`](openraft::membership::RaftMembership::read_quorum_set) is the
/// [`GridRule::FullRow`] rule of config i, and rule i of
/// [`write_quorum_set()`](openraft::membership::RaftMembership::write_quorum_set) is its
/// [`GridRule::OnePerRow`] rule. Both share the grid of config i. A change of the voter placement
/// or of the dimensions therefore keeps both grids until the joint reduces to its target. OpenRaft
/// changes a config without a joint only when both memberships hold a config with the same grid, so
/// every new grid goes through a joint.
///
/// [`grid_membership`](crate::docs::grid) explains how to choose the grid and how to change it.
///
/// # Node roles
///
/// A [`NodeRole`] says whether a node may start an election and whether it receives logs. A config
/// gives each of its voters [`NodeRole::Campaigner`] `|` [`NodeRole::ReceiveLogs`] and every other
/// node [`NodeRole::ReceiveLogs`], unless it sets another role. [`with_roles()`](Self::with_roles)
/// sets roles in a membership, and the `roles` of a [`GridChange`](crate::GridChange) set
/// them in its result. A voter with only [`NodeRole::ReceiveLogs`] stores logs and votes, but
/// starts no election on its own; a leadership transfer from the leader can still make it leader.
///
/// The grid alone decides which nodes are voters. Both rules count every row, so each voter is a
/// read voter and a write voter. Every voter therefore needs [`NodeRole::ReceiveLogs`], or a
/// write quorum that holds the voter never commits.
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
/// use openraft_membership::GridMembership;
///
/// // Reads need row {1,2,3} or row {4,5,6}. Writes need a voter of each row, from any column.
/// let grid = vec![vec![1, 2, 3], vec![4, 5, 6]];
/// let nodes = BTreeSet::from([1, 2, 3, 4, 5, 6]);
/// let membership = GridMembership::<u64, ()>::new(grid, nodes).unwrap();
///
/// let read_accepts = membership.read_quorum_set().is_quorum([4, 5, 6].iter());
/// assert!(read_accepts);
/// let write_accepts = membership.write_quorum_set().is_quorum([1, 5].iter());
/// assert!(write_accepts);
/// let write_accepts = membership.write_quorum_set().is_quorum([3, 4].iter());
/// assert!(write_accepts);
/// // {1,2,3} holds no voter of row {4,5,6}.
/// let write_accepts = membership.write_quorum_set().is_quorum([1, 2, 3].iter());
/// assert!(!write_accepts);
/// ```
#[since(version = "0.10.0")]
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct GridMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) configs: Vec<Config<NID, N, GridQuorumSet<NID>>>,
}

impl<NID, N> fmt::Debug for GridMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("GridMembership");
        debug.field("configs", &self.configs);
        debug.finish()
    }
}

/// Written by hand: `derive(Default)` would require `NID: Default` and `N: Default`.
impl<NID, N> Default for GridMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn default() -> Self {
        Self { configs: Vec::new() }
    }
}

/// Makes every node a voter and fills the most square grid row by row, in ascending ID order.
///
/// The grid has `r` rows of `c` voters, where `r * c` is the number of nodes, `r <= c`, and `r` is
/// the largest such divisor. Six nodes give `[[1,2,3],[4,5,6]]`. A prime number of nodes gives one
/// row, so every election needs every voter, and any one voter commits an entry.
impl<NID, N> From<BTreeMap<NID, N>> for GridMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn from(nodes: BTreeMap<NID, N>) -> Self {
        let node_ids = nodes.keys();
        let voters: Vec<NID> = node_ids.cloned().collect();
        if voters.is_empty() {
            return Self::default();
        }

        let voter_count = voters.len();
        let mut row_count = voter_count.isqrt();
        while !voter_count.is_multiple_of(row_count) {
            row_count -= 1;
        }
        let column_count = voter_count / row_count;

        let rows = voters.chunks(column_count);
        let grid: Vec<Vec<NID>> = rows.map(|row| row.to_vec()).collect();

        // Safe unwrap(): the grid has `row_count` rows of `column_count` distinct voters.
        let (read, write) = Self::rules_from_grid(grid).unwrap();
        // Safe unwrap(): every voter is a node, and the config sets no role.
        let config = Config::new(read, write, BTreeMap::new(), nodes).unwrap();
        Self { configs: vec![config] }
    }
}

impl<NID, N> fmt::Display for GridMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{configs: {:?}}}", self.configs)
    }
}

impl<NID, N> GridMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Builds a membership with one config that places its voters in `grid`, given as its rows.
    ///
    /// Every voter, an ID in a cell of `grid`, needs a node in `nodes`, and the other nodes in
    /// `nodes` are learners. `nodes` implements [`IntoNodes`], so it can be a `BTreeMap<NID, N>`,
    /// a `HashMap<NID, N>`, or a `BTreeSet<NID>` when `N` is `()` or
    /// [`EmptyNode`](openraft::EmptyNode).
    ///
    /// Returns [`MembershipError::UserDefined`] unless `grid` has a row and a column, every
    /// row is as long as the first, and no voter is in two cells. Returns
    /// [`MembershipError::NodeNotFound`] for a voter with no node.
    pub fn new<T>(grid: Vec<Vec<NID>>, nodes: T) -> Result<Self, MembershipError<NID, InvalidGrid<NID>>>
    where T: IntoNodes<NID, N> {
        let (read, write) = Self::rules_from_grid(grid)?;

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
    /// use openraft_membership::GridMembership;
    ///
    /// // Reads need row {1,2} or row {3,4}. Writes need a voter of each row, such as {1,4}.
    /// let grid = vec![vec![1, 2], vec![3, 4]];
    /// let nodes = BTreeSet::from([1, 2, 3, 4]);
    /// let membership = GridMembership::<u64, ()>::new(grid, nodes).unwrap();
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
    pub fn with_roles(self, roles: BTreeMap<NID, NodeRole>) -> Result<Self, MembershipError<NID, InvalidGrid<NID>>> {
        let configs = Config::with_roles(self.configs, roles)?;
        Ok(Self { configs })
    }

    /// Returns the `FullRow` read rule and the `OnePerRow` write rule of `grid`, checked like
    /// [`new()`](Self::new).
    pub(crate) fn rules_from_grid(
        grid: Vec<Vec<NID>>,
    ) -> Result<(GridQuorumSet<NID>, GridQuorumSet<NID>), InvalidGrid<NID>> {
        let read = GridQuorumSet::new(grid.clone(), GridRule::FullRow)?;
        let write = GridQuorumSet::new(grid, GridRule::OnePerRow)?;
        Ok((read, write))
    }
}

impl<NID, N> RaftMembership for GridMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type NodeId = NID;
    type Node = N;
    type QuorumSet = Joint<GridQuorumSet<NID>>;

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
