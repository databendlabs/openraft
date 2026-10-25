use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft_macros::since;

/// The write quorum size of a [`ThresholdMembership`] config does not fit its voters.
///
/// [`ThresholdMembership`] returns it inside [`ThresholdMembershipError::InvalidThresholds`].
/// Edits return it in [`MembershipError::UserDefined`]. OpenRaft wraps it with the configured
/// error source when applying an edit inside the Raft core.
///
/// [`ThresholdMembershipError::InvalidThresholds`]: crate::errors::ThresholdMembershipError::InvalidThresholds
///
/// [`ThresholdMembership`]: crate::ThresholdMembership
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

impl<NID> From<InvalidThresholds> for MembershipError<NID, InvalidThresholds>
where NID: NodeId
{
    fn from(e: InvalidThresholds) -> Self {
        MembershipError::UserDefined(e)
    }
}
