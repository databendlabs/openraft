use crate::RaftTypeConfig;
use crate::StorageError;
use crate::errors::NetworkError;
use crate::errors::RPCError;
use crate::errors::ReplicationClosed;
use crate::errors::ReplicationError;
use crate::errors::Timeout;
use crate::errors::Unreachable;

/// Error returned when streaming a snapshot to a remote Raft node, such as by
/// [`NetSnapshot::full_snapshot`].
///
/// - `Closed`: the transfer stopped because the `cancel` future given to the transport resolved.
/// - `StorageError`: reading the local snapshot failed. It is a failure of this node's storage, and
///   it stops this node.
/// - `Timeout`, `Unreachable`, and `Network`: transport failures, such as a timeout or a lost
///   connection.
///
/// There is no variant for an error returned by the remote node: this error converts only from an
/// [`RPCError`] whose remote error type is the default `Infallible`. A transport reports a failure
/// of the remote node, such as a target that could not install the snapshot, as one of the
/// transport failures, never as a `StorageError`.
///
/// [`NetSnapshot::full_snapshot`]: crate::network::NetSnapshot::full_snapshot
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(bound = ""))]
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

    /// The node is unreachable, and a retry should wait; see [`Unreachable`].
    #[error(transparent)]
    Unreachable(#[from] Unreachable<C>),

    /// A transient transport failure, which a retry may get past; see [`NetworkError`].
    #[error(transparent)]
    Network(#[from] NetworkError<C>),
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
