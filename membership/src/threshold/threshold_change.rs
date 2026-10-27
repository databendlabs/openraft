use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use display_more::DisplayOptionExt;
use openraft::Node;
use openraft::NodeId;
use openraft::errors::EmptyMembership;
use openraft::errors::ErrorSource;
use openraft::errors::MembershipError;
use openraft::impls::ChangeMembers;
use openraft::impls::Membership;
use openraft::membership::RaftMembershipEdit;
use openraft_macros::since;

use crate::ThresholdMembership;

/// A change to a [`ThresholdMembership`]. [`apply()`](RaftMembershipEdit::apply) applies `members`
/// first, then the write size, and derives the read size from the new voter count.
///
/// A change keeps the write size unless it sets it. When removing voters leaves fewer voters
/// than the write size, the change must also set a smaller write size. Otherwise `change()` returns
/// [`MembershipError::UserDefined`].
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
///
/// use openraft::impls::ChangeMembers;
/// use openraft_membership::{ThresholdChange, ThresholdMembership};
///
/// let voters = BTreeSet::from([1, 2, 3, 4, 5]);
/// let membership = ThresholdMembership::<u64, ()>::new(voters.clone(), 2, voters).unwrap();
///
/// // Require 4 copies to commit; the read size becomes 2.
/// let switched = membership.change(ThresholdChange::write_size(4)).unwrap();
/// let want = "{voters:[{1:(),2:(),3:(),4:(),5:()} read=2 write=4], learners:[]}";
/// assert_eq!(want, switched.to_string());
///
/// // Remove voter 5 and require 3 copies. The read size stays 2; node 5 stays as a learner.
/// let remove_five = ChangeMembers::RemoveVoters(BTreeSet::from([5]));
/// let change = ThresholdChange::members(remove_five).with_write_size(3);
/// let shrunk = switched.change(change).unwrap();
/// let want = "{voters:[{1:(),2:(),3:(),4:()} read=2 write=3], learners:[5:()]}";
/// assert_eq!(want, shrunk.to_string());
/// ```
///
/// [`ThresholdMembership`]: crate::ThresholdMembership
/// [`change()`]: crate::ThresholdMembership::change
/// [`MembershipError::UserDefined`]: openraft::errors::MembershipError::UserDefined
#[since(version = "0.10.0", change = "moved to the openraft-membership crate")]
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThresholdChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Voter and node changes; `None` keeps the voters and the nodes.
    pub members: Option<ChangeMembers<NID, N>>,

    /// The new write size; `None` keeps the current one.
    pub write_size: Option<u64>,
}

impl<NID, N> ThresholdChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns a change of the voters or the nodes that keeps the write size.
    pub fn members(change: impl Into<ChangeMembers<NID, N>>) -> Self {
        let members = change.into();
        Self {
            members: Some(members),
            write_size: None,
        }
    }

    /// Returns a change of the write size that keeps the voters and the nodes.
    pub fn write_size(write_size: u64) -> Self {
        Self {
            members: None,
            write_size: Some(write_size),
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

impl<NID, N> From<ChangeMembers<NID, N>> for ThresholdChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn from(members: ChangeMembers<NID, N>) -> Self {
        Self::members(members)
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
            "{{members: {}, write_size: {}}}",
            self.members.display(),
            self.write_size.display()
        )
    }
}

/// Each edit changes only the members, as the same edit of [`ChangeMembers`] does, and keeps both
/// sizes.
impl<NID, N, E> RaftMembershipEdit<ThresholdMembership<NID, N>, E> for ThresholdChange<NID, N>
where
    NID: NodeId,
    N: Node,
    E: ErrorSource,
{
    fn apply(
        self,
        membership: &ThresholdMembership<NID, N>,
    ) -> Result<ThresholdMembership<NID, N>, MembershipError<NID, E>> {
        tracing::debug!("ThresholdChange::apply: change: {}", self);

        let Some(last_write) = membership.write.rules().last() else {
            return Err(EmptyMembership {}.into());
        };

        let (voters, nodes) = match self.members {
            None => (last_write.id_set().clone(), membership.nodes.clone()),
            Some(members) => {
                let last_voters = last_write.id_set();
                let last_voters = last_voters.as_ref().clone();
                let current = Membership::<NID, N>::new(vec![last_voters], membership.nodes.clone())?;
                let target =
                    <ChangeMembers<NID, N> as RaftMembershipEdit<Membership<NID, N>, E>>::apply(members, &current)?;
                let (mut configs, nodes) = target.into_parts();
                let voters = configs.pop().unwrap();
                (Arc::new(voters), nodes)
            }
        };

        let write_size = self.write_size.unwrap_or(last_write.quorum_size());
        let voter_count = voters.len() as u64;
        let write_size = ThresholdMembership::<NID, N>::check_write_size(voter_count, write_size)?;
        let target = ThresholdMembership::from_configs([(voters, write_size)], nodes);

        tracing::debug!("target membership: {}", target);
        Ok(target)
    }

    fn add_learner(node_id: NID, node: N) -> Self {
        let nodes = BTreeMap::from([(node_id, node)]);
        let members = ChangeMembers::AddNodes(nodes);
        Self::members(members)
    }

    fn make_uniform() -> Self {
        let members = ChangeMembers::AddVoterIds(BTreeSet::new());
        Self::members(members)
    }
}
