use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft::errors::NodeNotFound;
use openraft_macros::since;

use crate::errors::InvalidThresholds;

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

impl<NID> From<ThresholdMembershipError<NID>> for MembershipError<NID, InvalidThresholds>
where NID: NodeId
{
    fn from(error: ThresholdMembershipError<NID>) -> Self {
        match error {
            ThresholdMembershipError::NodeNotFound(error) => Self::NodeNotFound(error),
            ThresholdMembershipError::InvalidThresholds(error) => Self::from(error),
        }
    }
}
