//! A module encapsulating the `RaftError` type & functionality.

use failure::Fail;

use crate::storage::StorageError;

/// Error variants which may come from the Raft actor.
#[derive(Debug, Fail)]
pub enum RaftError {
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
