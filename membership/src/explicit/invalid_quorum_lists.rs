use std::collections::BTreeSet;

use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft_macros::since;

/// The quorum lists of an [`ExplicitMembership`] config do not form usable rules whose read
/// quorums meet every write quorum.
///
/// [`ExplicitMembership`] and its edits return it in [`MembershipError::UserDefined`]. OpenRaft
/// wraps it with the configured error source when applying an edit inside the Raft core.
///
/// [`ExplicitMembership`]: crate::ExplicitMembership
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidQuorumLists<NID>
where NID: NodeId
{
    /// The read list holds no quorum, so no set is a read quorum.
    #[error("the read quorum list holds no quorum")]
    NoReadQuorum,

    /// The write list holds no quorum, so no set is a write quorum.
    #[error("the write quorum list holds no quorum")]
    NoWriteQuorum,

    /// The read list holds the empty set, so every set would be a read quorum.
    #[error("the read quorum list holds an empty quorum")]
    EmptyReadQuorum,

    /// The write list holds the empty set, so every set would be a write quorum.
    #[error("the write quorum list holds an empty quorum")]
    EmptyWriteQuorum,

    /// A read quorum and a write quorum share no node, so a read could miss a committed write.
    #[error("read quorum {read:?} and write quorum {write:?} share no node")]
    DisjointQuorums {
        /// The first listed read quorum that misses a write quorum.
        read: BTreeSet<NID>,

        /// The first listed write quorum that `read` misses.
        write: BTreeSet<NID>,
    },
}

impl<NID> From<InvalidQuorumLists<NID>> for MembershipError<NID, InvalidQuorumLists<NID>>
where NID: NodeId
{
    fn from(e: InvalidQuorumLists<NID>) -> Self {
        MembershipError::UserDefined(e)
    }
}
