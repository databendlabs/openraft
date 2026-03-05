use crate::RaftTypeConfig;
use crate::errors::ChangeMembershipError;
use crate::errors::EmptyMembership;
use crate::errors::LearnerNotFound;
use crate::errors::NodeNotFound;

/// Errors occur when building a [`Membership`].
///
/// [`Membership`]: crate::membership::Membership
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum MembershipError<C: RaftTypeConfig> {
    /// The membership configuration is empty.
    #[error(transparent)]
    EmptyMembership(#[from] EmptyMembership),

    /// A required node was not found.
    #[error(transparent)]
    NodeNotFound(#[from] NodeNotFound<C>),
}

impl<C> From<MembershipError<C>> for ChangeMembershipError<C>
where C: RaftTypeConfig
{
    fn from(me: MembershipError<C>) -> Self {
        match me {
            MembershipError::EmptyMembership(e) => ChangeMembershipError::EmptyMembership(e),
            MembershipError::NodeNotFound(e) => {
                ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: e.node_id })
            }
        }
    }
}
