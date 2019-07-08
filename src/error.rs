//! A module encapsulating the `RaftError` type & functionality.

use failure::Fail;

/// Error variants which may come from the Raft actor while handling Raft specifc RPCs.
#[derive(Debug, Fail)]
pub enum RaftRpcError {
    /// The Raft node is still initializing and couldn't handle the request.
    #[fail(display="Raft node is still initializing.")]
    Initializing,

    /// The Raft node encountered an internal messaging error.
    ///
    /// This type of error is related to the underlying actix framework and will be quite rare to
    /// come by. **This should be considered a transient type of error.**
    #[fail(display="Raft node encountered an internal messaging error.")]
    InternalMessagingError,

    /// An RPC was received from a node which does not appear to be a member of the cluster.
    #[fail(display="Raft node received an RPC frame from an unknown node.")]
    RPCFromUnknownNode,

    /// An error coming from the storage layer.
    #[fail(display="{}", _0)]
    StorageError(StorageError),

    /// The Raft node received an unknown request type.
    #[fail(display="Raft node received an unknown request type.")]
    UnknownRequestReceived,

    /// The Raft node received an append entries request while it was already appending entries from a previous request.
    #[fail(display="Raft node was already appending log entries.")]
    AppendEntriesAlreadyInProgress,
}

/// Error variants which may come from the Raft actor while handling client requests.
#[derive(Debug, Fail)]
pub enum ClientRpcError {
    /// An error has taken place while attempting to forward a client RPC to the cluster leader.
    ///
    /// This error indicates the the RPC was not successfully forwarded, so the RPC was never
    /// successfully sent to the target node.
    #[fail(display="Raft failed to forward request to cluster leader.")]
    ForwardingError,

    /// The Raft node encountered an internal messaging error.
    ///
    /// This type of error is related to the underlying actix framework and will be quite rare to
    /// come by. **This should be considered a transient type of error.**
    #[fail(display="Raft node encountered an internal messaging error.")]
    InternalMessagingError,

    /// An error coming from the storage layer.
    #[fail(display="{}", _0)]
    StorageError(StorageError),
}

/// An error type which wraps an application specific error type coming from the storage layer.
///
/// The intention is that applications which are using this crate will be able to pass their own
/// concrete error types up from the storage layer, through the Raft system, to the higher levels
/// of their application for more granular control. Many applications will need to be able to
/// communicate application specific logic from the storage layer.
///
/// **NOTE WELL:** if a `StorageError` is returned from any of the `RaftStorage` interfaces, other
/// than the `AppendLogEntries` interface while the Raft node is in **leader state**, then the Raft
/// node will shutdown. This is due to the fact that custom error handling logic is only allowed
/// in the `AppendLogEntries` interface while the Raft node is the cluster leader. When the node
/// is in any other state, the storage layer is expected to operate without any errors.
#[derive(Debug, Fail)]
#[fail(display="{}", _0)]
pub struct StorageError(pub Box<dyn Fail>);

/// The result type of all `RaftStorage` interfaces.
pub type StorageResult<T> = Result<T, StorageError>;
