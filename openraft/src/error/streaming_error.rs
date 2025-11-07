use crate::RaftTypeConfig;
use crate::StorageError;
use crate::error::NetworkError;
use crate::error::RPCError;
use crate::error::ReplicationClosed;
use crate::error::ReplicationError;
use crate::error::Timeout;
use crate::error::Unreachable;

/// Error occurred when streaming local data to a remote raft node.
///
/// Thus, this error includes storage error, network error, and remote error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum StreamingError<C: RaftTypeConfig> {
    /// The replication stream is closed intentionally.
    #[error(transparent)]
    Closed(#[from] ReplicationClosed),

    /// Storage error occurs when reading local data.
    #[error(transparent)]
    StorageError(#[from] StorageError<C>),

    /// Timeout when streaming data to the remote node.
    #[error(transparent)]
    Timeout(#[from] Timeout<C>),

    /// The node is temporarily unreachable and should backoff before retrying.
    #[error(transparent)]
    Unreachable(#[from] Unreachable),

    /// Failed to send the RPC request and should retry immediately.
    #[error(transparent)]
    Network(#[from] NetworkError),
}

impl<C: RaftTypeConfig> From<StreamingError<C>> for ReplicationError<C> {
    fn from(e: StreamingError<C>) -> Self {
        match e {
            StreamingError::Closed(e) => ReplicationError::Closed(e),
            StreamingError::StorageError(e) => ReplicationError::StorageError(e),
            StreamingError::Timeout(e) => ReplicationError::RPCError(RPCError::Timeout(e)),
            StreamingError::Unreachable(e) => ReplicationError::RPCError(RPCError::Unreachable(e)),
            StreamingError::Network(e) => ReplicationError::RPCError(RPCError::Network(e)),
        }
    }
}

impl<C: RaftTypeConfig> From<RPCError<C>> for StreamingError<C> {
    fn from(value: RPCError<C>) -> Self {
        match value {
            RPCError::Timeout(e) => StreamingError::Timeout(e),
            RPCError::Unreachable(e) => StreamingError::Unreachable(e),
            RPCError::Network(e) => StreamingError::Network(e),
        }
    }
}
