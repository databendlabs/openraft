use crate::RaftTypeConfig;
use crate::StorageError;
use crate::error::RPCError;
use crate::error::higher_vote::HigherVote;
use crate::error::replication_closed::ReplicationClosed;

/// Error variants related to the Replication.
#[derive(Debug, thiserror::Error)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ReplicationError<C>
where C: RaftTypeConfig
{
    #[error(transparent)]
    HigherVote(#[from] HigherVote<C>),

    #[error(transparent)]
    Closed(#[from] ReplicationClosed),

    #[error(transparent)]
    StorageError(#[from] StorageError<C>),

    #[error(transparent)]
    RPCError(#[from] RPCError<C>),
}
