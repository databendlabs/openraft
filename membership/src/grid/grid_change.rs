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

use crate::GridMembership;
use crate::config::Config;
use crate::errors::InvalidGrid;

/// A change to a [`GridMembership`]. [`apply()`](RaftMembershipEdit::apply) adds `add_nodes`, sets
/// the grid, removes `remove_nodes`, then sets `roles`.
///
/// A change keeps the grid unless it sets one, and keeps the roles that the last config sets for
/// the nodes that remain. A new grid can move voters to other cells, change the dimensions, or
/// both. An invalid grid makes `apply()` return [`MembershipError::UserDefined`]. A voter without a
/// node makes `apply()` return [`MembershipError::NodeNotFound`].
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
/// use openraft_membership::{GridChange, GridMembership};
///
/// let grid = vec![vec![1, 2], vec![3, 4]];
/// let nodes = BTreeSet::from([1, 2, 3, 4, 5, 6]);
/// let membership = GridMembership::<u64, ()>::new(grid, nodes).unwrap();
///
/// // Learners 5 and 6 become a third column.
/// let change = GridChange::grid(vec![vec![1, 2, 5], vec![3, 4, 6]]);
/// let wider = change.apply(&membership, true).unwrap();
/// let want = "{configs: [Config { read: GridQuorumSet { grid: [[1, 2, 5], [3, 4, 6]], line: Row }, \
///             write: GridQuorumSet { grid: [[1, 2, 5], [3, 4, 6]], line: Column }, \
///             nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
///             3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
///             5: ((), NodeRole(Campaigner | ReceiveLogs)), 6: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
/// let got = wider.to_string();
/// assert_eq!(want, got);
/// ```
///
/// [`MembershipError::UserDefined`]: openraft::errors::MembershipError::UserDefined
/// [`MembershipError::NodeNotFound`]: openraft::errors::MembershipError::NodeNotFound
/// [`MembershipError::WriteVoterWithoutReceiveLogs`]: openraft::errors::MembershipError::WriteVoterWithoutReceiveLogs
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Nodes to add as learners. A node that already exists keeps its metadata.
    pub add_nodes: BTreeMap<NID, N>,

    /// The new grid, as its rows of voters; `None` keeps the grid.
    pub grid: Option<Vec<Vec<NID>>>,

    /// Nodes to remove. Removing a voter of the result fails.
    pub remove_nodes: BTreeSet<NID>,

    /// The roles to set in the result, by node ID; each node must be a node of the result. A node
    /// that it does not name keeps the role that the last config sets.
    pub roles: BTreeMap<NID, NodeRole>,
}

impl<NID, N> GridChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns a change of the grid that keeps the nodes.
    pub fn grid(grid: Vec<Vec<NID>>) -> Self {
        Self {
            grid: Some(grid),
            ..Self::default()
        }
    }

    /// Returns this change with `roles` as the roles that it sets.
    pub fn with_roles(self, roles: BTreeMap<NID, NodeRole>) -> Self {
        Self { roles, ..self }
    }
}

impl<NID, N> Default for GridChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns the empty change, which keeps the grid and the nodes.
    fn default() -> Self {
        Self {
            add_nodes: BTreeMap::new(),
            grid: None,
            remove_nodes: BTreeSet::new(),
            roles: BTreeMap::new(),
        }
    }
}

impl<NID, N> fmt::Display for GridChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{add_nodes: {:?}, grid: {}, remove_nodes: {:?}, roles: {:?}}}",
            self.add_nodes,
            self.grid.display_debug(),
            self.remove_nodes,
            self.roles
        )
    }
}

/// `add_learner()` adds one node and keeps the grid.
/// `make_uniform()` returns the empty change, which reduces a joint membership to its last config.
impl<NID, N> RaftMembershipEdit<GridMembership<NID, N>> for GridChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type Error = InvalidGrid<NID>;

    fn apply(
        self,
        membership: &GridMembership<NID, N>,
        retain: bool,
    ) -> Result<GridMembership<NID, N>, MembershipError<NID, Self::Error>> {
        tracing::debug!("GridChange::apply: change: {}", self);

        let Some(last) = membership.configs.last() else {
            return Err(EmptyMembership {}.into());
        };

        let (read, write) = match self.grid {
            None => (last.read().clone(), last.write().clone()),
            Some(grid) => GridMembership::<NID, N>::rules_from_grid(grid)?,
        };

        let nodes = Config::edit_nodes(&membership.configs, self.add_nodes, &self.remove_nodes);
        let roles = last.edit_roles(self.roles, &nodes)?;
        let config = Config::new(read, write, roles, nodes)?;

        let mut target = GridMembership { configs: vec![config] };
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
