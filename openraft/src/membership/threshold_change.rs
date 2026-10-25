use std::collections::BTreeSet;
use std::fmt;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::ChangeMembers;
use crate::Node;
use crate::NodeId;
use crate::membership::RaftMembershipEdit;

/// A change to a [`ThresholdMembership`]. [`change()`] applies `members` first, then the sizes,
/// and checks the result.
///
/// A change keeps both sizes unless it sets them, so no change weakens or strengthens a quorum
/// unless the application asks. When a change of the voters makes the current sizes invalid for
/// the new voter count, it must also set new sizes. Otherwise `change()` returns
/// [`InvalidThresholds`] inside [`MembershipError::UserDefined`].
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
///
/// use openraft::ChangeMembers;
/// use openraft::membership::{RaftMembership, ThresholdChange, ThresholdMembership};
///
/// let voters = BTreeSet::from([1, 2, 3, 4, 5]);
/// let membership = ThresholdMembership::<u64, ()>::new(voters.clone(), 4, 2, voters).unwrap();
///
/// // Switch to read_size = 2 and write_size = 4.
/// let switched = membership.change(ThresholdChange::sizes(2, 4)).unwrap();
/// let want = "{voters:[{1:(),2:(),3:(),4:(),5:()} read=2 write=4], learners:[]}";
/// assert_eq!(want, switched.to_string());
///
/// // Remove voter 5, with sizes that fit 4 voters. Node 5 stays as a learner.
/// let remove_five = ChangeMembers::RemoveVoters(BTreeSet::from([5]));
/// let change = ThresholdChange::members(remove_five).with_sizes(2, 3);
/// let shrunk = switched.change(change).unwrap();
/// let want = "{voters:[{1:(),2:(),3:(),4:()} read=2 write=3], learners:[5:()]}";
/// assert_eq!(want, shrunk.to_string());
/// ```
///
/// [`ThresholdMembership`]: crate::membership::ThresholdMembership
/// [`change()`]: crate::membership::RaftMembership::change
/// [`InvalidThresholds`]: crate::membership::InvalidThresholds
/// [`MembershipError::UserDefined`]: crate::errors::MembershipError::UserDefined
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThresholdChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Voter and node changes; `None` keeps the voters and the nodes.
    pub members: Option<ChangeMembers<NID, N>>,

    /// The new read size; `None` keeps the current one.
    pub read_size: Option<u64>,

    /// The new write size; `None` keeps the current one.
    pub write_size: Option<u64>,
}

impl<NID, N> ThresholdChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Returns a change of the voters or the nodes that keeps both sizes.
    pub fn members(change: impl Into<ChangeMembers<NID, N>>) -> Self {
        let members = change.into();
        Self {
            members: Some(members),
            read_size: None,
            write_size: None,
        }
    }

    /// Returns a change of both sizes that keeps the voters and the nodes.
    pub fn sizes(read_size: u64, write_size: u64) -> Self {
        Self {
            members: None,
            read_size: Some(read_size),
            write_size: Some(write_size),
        }
    }

    /// Returns this change with both sizes set.
    pub fn with_sizes(self, read_size: u64, write_size: u64) -> Self {
        Self {
            read_size: Some(read_size),
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
            "{{members: {}, read_size: {}, write_size: {}}}",
            self.members.display(),
            self.read_size.display(),
            self.write_size.display()
        )
    }
}

/// Each edit changes only the members, as the same edit of [`ChangeMembers`] does, and keeps both
/// sizes.
impl<NID, N> RaftMembershipEdit for ThresholdChange<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type NodeId = NID;
    type Node = N;

    fn add_learner(node_id: NID, node: N) -> Self {
        let members = ChangeMembers::<NID, N>::add_learner(node_id, node);
        Self::members(members)
    }

    fn replace_all_voters(voter_ids: BTreeSet<NID>) -> Self {
        let members = ChangeMembers::<NID, N>::replace_all_voters(voter_ids);
        Self::members(members)
    }

    fn leave_joint() -> Self {
        let members = ChangeMembers::<NID, N>::leave_joint();
        Self::members(members)
    }
}
