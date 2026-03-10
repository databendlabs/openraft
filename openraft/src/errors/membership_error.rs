use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::errors::ChangeMembershipError;
use crate::errors::EmptyMembership;
use crate::errors::LearnerNotFound;
use crate::errors::NodeNotFound;
use crate::node::NodeId;

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

impl<C> From<MembershipError<C::NodeId>> for ChangeMembershipError<C>
where C: RaftTypeConfig
{
    fn from(me: MembershipError<C::NodeId>) -> Self {
        match me {
            MembershipError::EmptyMembership(e) => ChangeMembershipError::EmptyMembership(e),
            MembershipError::NodeNotFound(e) => {
                ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: e.node_id })
            }
        }
    }
}
