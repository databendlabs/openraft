use anyerror::AnyError;
use openraft_macros::since;

use crate::NodeId;
use crate::errors::MembershipError;

/// The write quorum size of a [`ThresholdMembership`] config does not fit its voters.
///
/// [`ThresholdMembership`] returns it inside [`MembershipError::UserDefined`].
///
/// [`ThresholdMembership`]: crate::membership::ThresholdMembership
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "invalid threshold: write_size={write_size}, voter_count={voter_count}; \
     need 1 <= write_size <= voter_count"
)]
pub struct InvalidThresholds {
    /// The number of voters of the config.
    pub voter_count: u64,

    /// The number of voters that a write quorum of the config must hold.
    pub write_size: u64,
}

impl<NID> From<InvalidThresholds> for MembershipError<NID>
where NID: NodeId
{
    fn from(e: InvalidThresholds) -> Self {
        let error = AnyError::new(&e);
        MembershipError::UserDefined(error)
    }
}
