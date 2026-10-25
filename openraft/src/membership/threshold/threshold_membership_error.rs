use openraft_macros::since;

use crate::NodeId;
use crate::errors::ErrorSource;
use crate::errors::MembershipError;
use crate::errors::NodeNotFound;
use crate::membership::InvalidThresholds;

/// A threshold membership has invalid sizes or missing voter metadata.
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ThresholdMembershipError<NID>
where NID: NodeId
{
    /// A voter has no node metadata.
    #[error(transparent)]
    NodeNotFound(#[from] NodeNotFound<NID>),

    /// The write quorum size does not fit the voters.
    #[error(transparent)]
    InvalidThresholds(#[from] InvalidThresholds),
}

impl<NID, E> From<ThresholdMembershipError<NID>> for MembershipError<NID, E>
where
    NID: NodeId,
    E: ErrorSource,
{
    fn from(error: ThresholdMembershipError<NID>) -> Self {
        match error {
            ThresholdMembershipError::NodeNotFound(error) => Self::NodeNotFound(error),
            ThresholdMembershipError::InvalidThresholds(error) => Self::from(error),
        }
    }
}
