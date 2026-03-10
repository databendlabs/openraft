use openraft_macros::since;

use crate::errors::ChangeMembershipError;
use crate::errors::EmptyMembership;
use crate::errors::LearnerNotFound;
use crate::errors::NodeNotFound;
use crate::node::NodeId;
use crate::vote::RaftCommittedLeaderId;

/// Errors occur when building a [`Membership`].
///
/// [`Membership`]: crate::membership::Membership
#[since(version = "0.10.0", change = "removed `C: RaftTypeConfig` generic parameter")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum MembershipError<NID>
where NID: NodeId
{
    /// The membership configuration is empty.
    #[error(transparent)]
    EmptyMembership(#[from] EmptyMembership),

    /// A required node was not found.
    #[error(transparent)]
    NodeNotFound(#[from] NodeNotFound<NID>),
}

impl<CLID, NID> From<MembershipError<NID>> for ChangeMembershipError<CLID, NID>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
{
    fn from(me: MembershipError<NID>) -> Self {
        match me {
            MembershipError::EmptyMembership(e) => ChangeMembershipError::EmptyMembership(e),
            MembershipError::NodeNotFound(e) => {
                ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: e.node_id })
            }
        }
    }
}
