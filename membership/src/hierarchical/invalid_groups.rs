use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft_macros::since;

/// The groups or the quorum sizes of a [`HierarchicalQuorumSet`] are not usable.
///
/// [`HierarchicalQuorumSet::new()`] returns it, and [`HierarchicalMembership`] and its edits
/// return it in [`MembershipError::UserDefined`]. The quorum sizes of a hierarchical config are
/// the write sizes of its groups. OpenRaft wraps it with the configured error source when applying
/// an edit inside the Raft core.
///
/// Each variant names a group by its path: the IDs of the groups from the root down to it. The
/// path of the root group is empty.
///
/// [`HierarchicalQuorumSet`]: crate::quorum::HierarchicalQuorumSet
/// [`HierarchicalQuorumSet::new()`]: crate::quorum::HierarchicalQuorumSet::new
/// [`HierarchicalMembership`]: crate::HierarchicalMembership
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidGroups<NID>
where NID: NodeId
{
    /// A group holds no voter and no child group, so no set reaches it.
    #[error("group {group:?} holds no member")]
    EmptyGroup {
        /// The path of the empty group.
        group: Vec<String>,
    },

    /// Two groups hold the same voter, but each voter belongs to exactly one group of the tree.
    #[error("groups {first:?} and {second:?} both hold voter {node_id}")]
    SharedVoter {
        /// The voter that both groups hold.
        node_id: NID,

        /// The path of the group that the check visits first. It visits a group before the groups
        /// below it, and sibling groups in ID order.
        first: Vec<String>,

        /// The path of the group that the check visits second.
        second: Vec<String>,
    },

    /// The number of members that a quorum must reach in a group is outside `1..=member_count`.
    #[error(
        "a quorum must reach {quorum_size} of the {member_count} members of group {group:?}; \
         need 1 <= quorum_size <= member_count"
    )]
    InvalidQuorumSize {
        /// The path of the group.
        group: Vec<String>,

        /// The number of members of the group that a quorum must reach.
        quorum_size: u64,

        /// The number of members of the group: its voters and its child groups.
        member_count: u64,
    },
}

impl<NID> From<InvalidGroups<NID>> for MembershipError<NID, InvalidGroups<NID>>
where NID: NodeId
{
    fn from(e: InvalidGroups<NID>) -> Self {
        MembershipError::UserDefined(e)
    }
}
