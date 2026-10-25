use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use display_more::DisplayDebugOptionExt;
use display_more::DisplayOptionExt;
use openraft::Node;
use openraft::NodeId;
use openraft::errors::EmptyMembership;
use openraft::errors::MembershipError;
use openraft::membership::RaftMembershipEdit;
use openraft_macros::since;

use crate::ThresholdMembership;
use crate::errors::InvalidThresholds;

/// A change to a [`ThresholdMembership`]. [`apply()`](RaftMembershipEdit::apply) adds
/// `add_nodes`, sets the voters and the write size, then removes `remove_nodes`. It derives the
/// read size from the new voter count.
///
/// A change keeps the write size unless it sets it. When removing voters leaves fewer voters
/// than the write size, the change must also set a smaller write size. Otherwise `apply()` returns
/// [`MembershipError::UserDefined`]. A voter without a node makes `apply()` return
/// [`MembershipError::NodeNotFound`].
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
///
/// use openraft::membership::RaftMembershipEdit;
/// use openraft_membership::{ThresholdChange, ThresholdMembership};
///
/// let voters = BTreeSet::from([1, 2, 3, 4, 5]);
/// let membership = ThresholdMembership::<u64, ()>::new(voters.clone(), 2, voters).unwrap();
///
/// // Require 4 copies to commit; the read size becomes 2.
/// let change = ThresholdChange::write_size(4);
/// let switched = change.apply(&membership).unwrap();
/// let want = "{voters:[{1:(),2:(),3:(),4:(),5:()} read=2 write=4], learners:[]}";
/// assert_eq!(want, switched.to_string());
///
/// // Remove voter 5 and require 3 copies. The read size stays 2; node 5 stays as a learner.
/// let change = ThresholdChange::voters(BTreeSet::from([1, 2, 3, 4])).with_write_size(3);
/// let shrunk = change.apply(&switched).unwrap();
/// let want = "{voters:[{1:(),2:(),3:(),4:()} read=2 write=3], learners:[5:()]}";
/// assert_eq!(want, shrunk.to_string());
/// ```
///
/// [`ThresholdMembership`]: crate::ThresholdMembership
/// [`MembershipError::UserDefined`]: openraft::errors::MembershipError::UserDefined
/// [`MembershipError::NodeNotFound`]: openraft::errors::MembershipError::NodeNotFound
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThresholdChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Nodes to add as learners. A node that already exists keeps its metadata.
    pub add_nodes: BTreeMap<NID, N>,

    /// The new voters; `None` keeps the voters.
    pub voters: Option<BTreeSet<NID>>,

    /// The new write size; `None` keeps the current one.
    pub write_size: Option<u64>,

    /// Nodes to remove. Removing a voter of the result fails.
    pub remove_nodes: BTreeSet<NID>,
}

impl<NID, N> ThresholdChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns a change of the voters that keeps the write size and the nodes.
    pub fn voters(voters: BTreeSet<NID>) -> Self {
        Self {
            voters: Some(voters),
            ..Self::default()
        }
    }

    /// Returns a change of the write size that keeps the voters and the nodes.
    pub fn write_size(write_size: u64) -> Self {
        Self {
            write_size: Some(write_size),
            ..Self::default()
        }
    }

    /// Returns this change with the write size set.
    pub fn with_write_size(self, write_size: u64) -> Self {
        Self {
            write_size: Some(write_size),
            ..self
        }
    }
}

impl<NID, N> Default for ThresholdChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns the empty change, which keeps the voters, the write size and the nodes.
    fn default() -> Self {
        Self {
            add_nodes: BTreeMap::new(),
            voters: None,
            write_size: None,
            remove_nodes: BTreeSet::new(),
        }
    }
}

impl<NID, N> fmt::Display for ThresholdChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{add_nodes: {:?}, voters: {}, write_size: {}, remove_nodes: {:?}}}",
            self.add_nodes,
            self.voters.display_debug(),
            self.write_size.display(),
            self.remove_nodes
        )
    }
}

/// `add_learner()` adds one node and keeps the voters and the write size.
/// `make_uniform()` returns the empty change, which reduces a joint membership to its last config.
impl<NID, N> RaftMembershipEdit<ThresholdMembership<NID, N>> for ThresholdChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type Error = InvalidThresholds;

    fn apply(
        self,
        membership: &ThresholdMembership<NID, N>,
    ) -> Result<ThresholdMembership<NID, N>, MembershipError<NID, Self::Error>> {
        tracing::debug!("ThresholdChange::apply: change: {}", self);

        let Some(last_write) = membership.write.rules().last() else {
            return Err(EmptyMembership {}.into());
        };

        let mut nodes = membership.nodes.clone();
        for (node_id, node) in self.add_nodes {
            nodes.entry(node_id).or_insert(node);
        }

        let voters = match self.voters {
            None => last_write.id_set().clone(),
            Some(voters) => Arc::new(voters),
        };

        let write_size = self.write_size.unwrap_or(last_write.quorum_size());
        let voter_count = voters.len() as u64;
        let write_size = ThresholdMembership::<NID, N>::check_write_size(voter_count, write_size)?;

        for node_id in &self.remove_nodes {
            nodes.remove(node_id);
        }

        let target = ThresholdMembership::from_configs([(voters, write_size)], nodes);
        target.ensure_voters_have_nodes()?;

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
