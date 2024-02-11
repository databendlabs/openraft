use std::error::Error;

use crate::error::NetworkError;
use crate::error::RPCError;
use crate::error::RaftError;
use crate::error::RemoteError;
use crate::error::ReplicationClosed;
use crate::error::ReplicationError;
use crate::error::Timeout;
use crate::error::Unreachable;
use crate::RaftTypeConfig;
use crate::StorageError;

/// Error occurs when streaming local data to a remote raft node.
///
/// Thus this error includes storage error, network error, and remote error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(bound(serialize = "E: serde::Serialize")),
    serde(bound(deserialize = "E: for <'d> serde::Deserialize<'d>"))
)]
pub enum StreamingError<C: RaftTypeConfig, E: Error> {
    /// The replication stream is closed intentionally.
    #[error(transparent)]
    Closed(#[from] ReplicationClosed),

    /// Storage error occurs when reading local data.
    #[error(transparent)]
    StorageError(#[from] StorageError<C::NodeId>),

    /// Timeout when streaming data to remote node.
    #[error(transparent)]
    Timeout(#[from] Timeout<C::NodeId>),

    /// The node is temporarily unreachable and should backoff before retrying.
    #[error(transparent)]
    Unreachable(#[from] Unreachable),

    /// Failed to send the RPC request and should retry immediately.
    #[error(transparent)]
    Network(#[from] NetworkError),

    /// Remote node returns an error.
    #[error(transparent)]
    RemoteError(#[from] RemoteError<C::NodeId, C::Node, E>),
}

impl<C: RaftTypeConfig, E> From<StreamingError<C, E>> for ReplicationError<C::NodeId, C::Node>
where
    E: Error,
    RaftError<C::NodeId>: From<E>,
{
    fn from(e: StreamingError<C, E>) -> Self {
        match e {
            StreamingError::Closed(e) => ReplicationError::Closed(e),
            StreamingError::StorageError(e) => ReplicationError::StorageError(e),
            StreamingError::Timeout(e) => ReplicationError::RPCError(RPCError::Timeout(e)),
            StreamingError::Unreachable(e) => ReplicationError::RPCError(RPCError::Unreachable(e)),
            StreamingError::Network(e) => ReplicationError::RPCError(RPCError::Network(e)),
            StreamingError::RemoteError(e) => {
                let remote_err = RemoteError {
                    target: e.target,
                    target_node: e.target_node,
                    source: RaftError::from(e.source),
                };
                ReplicationError::RPCError(RPCError::RemoteError(remote_err))
            }
        }
    }
}
