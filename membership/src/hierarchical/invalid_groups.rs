use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft_macros::since;

/// The groups or the thresholds of a [`HierarchicalQuorumSet`] are not usable.
///
/// [`HierarchicalQuorumSet::new()`] returns it, and [`HierarchicalMembership`] and its edits
/// return it in [`MembershipError::UserDefined`]. The `write_groups` and the write sizes of a
/// hierarchical config are the `quorum_groups` and the quorum sizes of its write rule. OpenRaft
/// wraps it with the configured error source when applying an edit inside the Raft core.
///
/// [`HierarchicalQuorumSet`]: crate::quorum::HierarchicalQuorumSet
/// [`HierarchicalQuorumSet::new()`]: crate::quorum::HierarchicalQuorumSet::new
/// [`HierarchicalMembership`]: crate::HierarchicalMembership
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidGroups<NID>
where NID: NodeId
{
    /// The layout holds no group, so no set is a quorum.
    #[error("the layout holds no group")]
    NoGroup,

    /// A group holds no voter, so no set reaches it.
    #[error("group {group} holds no voter")]
    EmptyGroup {
        /// The ID of the empty group.
        group: String,
    },

    /// Two groups hold the same voter, but each voter belongs to exactly one group.
    #[error("groups {first} and {second} both hold voter {node_id}")]
    SharedVoter {
        /// The voter that both groups hold.
        node_id: NID,

        /// The group with the smaller ID.
        first: String,

        /// The group with the larger ID.
        second: String,
    },

    /// The number of groups that a quorum must reach is outside `1..=group_count`.
    #[error(
        "a quorum must reach {quorum_groups} of {group_count} groups; \
         need 1 <= quorum_groups <= group_count"
    )]
    InvalidQuorumGroups {
        /// The number of groups that a quorum must reach.
        quorum_groups: u64,

        /// The number of groups of the layout.
        group_count: u64,
    },

    /// A quorum size names a group that the layout lacks. Only the write sizes of a
    /// [`HierarchicalMembership`](crate::HierarchicalMembership) or a
    /// [`HierarchicalChange`](crate::HierarchicalChange), given separately from the groups, can do
    /// so.
    #[error("a quorum size names group {group}, which the layout lacks")]
    UnknownGroup {
        /// The ID that names no group of the layout.
        group: String,
    },

    /// A group of the layout has no quorum size. Only the write sizes of a
    /// [`HierarchicalMembership`](crate::HierarchicalMembership) or a
    /// [`HierarchicalChange`](crate::HierarchicalChange), given separately from the groups, can
    /// miss a group.
    #[error("group {group} has no quorum size")]
    MissingQuorumSize {
        /// The ID of the group without a quorum size.
        group: String,
    },

    /// The number of voters that a quorum must hold in a group is outside `1..=voter_count`.
    #[error(
        "a quorum must hold {quorum_size} of the {voter_count} voters of group {group}; \
         need 1 <= quorum_size <= voter_count"
    )]
    InvalidQuorumSize {
        /// The ID of the group.
        group: String,

        /// The number of voters of the group that a quorum must hold.
        quorum_size: u64,

        /// The number of voters of the group.
        voter_count: u64,
    },
}

impl<NID> From<InvalidGroups<NID>> for MembershipError<NID, InvalidGroups<NID>>
where NID: NodeId
{
    fn from(e: InvalidGroups<NID>) -> Self {
        MembershipError::UserDefined(e)
    }
}
