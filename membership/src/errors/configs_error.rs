use std::error::Error;

use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft::errors::NodeNotFound;
use openraft::membership::errors::WriteVoterWithoutReceiveLogs;

/// The configs of a membership have a voter without node metadata, or a write voter that does not
/// receive logs in its config.
///
/// [`MembershipError`] converts it into its variant of the same name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ConfigsError<NID>
where NID: NodeId
{
    /// A voter, or a node whose role is set, has no node metadata.
    #[error(transparent)]
    NodeNotFound(#[from] NodeNotFound<NID>),

    /// A write voter lacks [`NodeRole::ReceiveLogs`] in a config, so a write quorum of that config
    /// that needs it never commits.
    ///
    /// [`NodeRole::ReceiveLogs`]: openraft::membership::NodeRole::ReceiveLogs
    #[error(transparent)]
    WriteVoterWithoutReceiveLogs(#[from] WriteVoterWithoutReceiveLogs<NID>),
}

impl<NID, E> From<ConfigsError<NID>> for MembershipError<NID, E>
where
    NID: NodeId,
    E: Error + 'static,
{
    fn from(error: ConfigsError<NID>) -> Self {
        match error {
            ConfigsError::NodeNotFound(error) => Self::NodeNotFound(error),
            ConfigsError::WriteVoterWithoutReceiveLogs(error) => Self::WriteVoterWithoutReceiveLogs(error),
        }
    }
}
