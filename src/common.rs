//! Common types and functionality used by the Raft actor.

use std::sync::Arc;

use futures::sync::oneshot;

use crate::{
    NodeId, AppError,
    messages::{
        ClientError, ClientPayload, ClientPayloadResponse,
        Entry, EntryNormal, EntryType, ResponseMode,
    },
};

pub(crate) const CLIENT_RPC_RX_ERR: &str = "Client RPC channel receiver was unexpectedly closed.";
pub(crate) const CLIENT_RPC_TX_ERR: &str = "Client RPC channel sender was unexpectedly closed.";

//////////////////////////////////////////////////////////////////////////////////////////////////
// ApplyLogsTask /////////////////////////////////////////////////////////////////////////////////

/// A task declaring some set of logs which need to be applied to the state machine.
pub(crate) enum ApplyLogsTask<E: AppError> {
    /// Check for & apply any logs which have been committed but which have not yet been applied.
    Outstanding,
    /// Logs which need to be applied are supplied for immediate use.
    Entry {
        /// The payload of logs to be applied.
        entry: Arc<Entry>,
        /// The optional response channel for when this task is complete.
        chan: Option<oneshot::Sender<Result<ClientPayloadResponse, ClientError<E>>>>,
    },
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ClientPayloadWithChan /////////////////////////////////////////////////////////////////////////

pub(crate) struct ClientPayloadWithChan<E: AppError> {
    pub tx: oneshot::Sender<Result<ClientPayloadResponse, ClientError<E>>>,
    pub rpc: ClientPayload<E>,
}

impl<E: AppError> ClientPayloadWithChan<E> {
    /// Upgrade a client payload with an assigned index & term.
    ///
    /// - `index`: the index to assign to this payload.
    /// - `term`: the term to assign to this payload.
    pub(crate) fn upgrade(self, index: u64, term: u64) -> ClientPayloadWithIndex<E> {
        ClientPayloadWithIndex::new(self, index, term)
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ClientPayloadWithIndex /////////////////////////////////////////////////////////////////////////

/// A client payload which has made its way into the processing pipeline.
pub(crate) struct ClientPayloadWithIndex<E: AppError> {
    /// The channel of the original client request.
    pub tx: oneshot::Sender<Result<ClientPayloadResponse, ClientError<E>>>,
    /// The original entry of this payload.
    orig_entry: EntryNormal,
    /// The entry of the original request with an assigned index & term, ready for storage.
    entry: Arc<Entry>,
    /// The response mode of the original client request.
    pub response_mode: ResponseMode,
    /// The assigned log index of this payload.
    pub index: u64,
    /// The term associated with this payload.
    pub term: u64,
}

impl<E: AppError> ClientPayloadWithIndex<E> {
    /// Create a new instance.
    pub(self) fn new(payload: ClientPayloadWithChan<E>, index: u64, term: u64) -> Self {
        let entry = Arc::new(Entry{index: index, term: term, entry_type: EntryType::Normal(payload.rpc.entry.clone())});
        Self{tx: payload.tx, orig_entry: payload.rpc.entry, entry, response_mode: payload.rpc.response_mode, index, term}
    }

    /// Downgrade the payload, typically for forwarding purposes.
    pub(crate) fn downgrade(self) -> ClientPayloadWithChan<E> {
        ClientPayloadWithChan{tx: self.tx, rpc: ClientPayload::new(self.orig_entry, self.response_mode)}
    }

    /// Get a reference to the entry encapsulated by this payload.
    pub(crate) fn entry(&self) -> Arc<Entry> {
        self.entry.clone()
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// DependencyAddr /////////////////////////////////////////////////////////////////////////////////

/// The set of dependency addr types used for tracking and reporting messaging errors.
#[derive(Debug)]
pub(crate) enum DependencyAddr {
    /// An addr of an internal actor which is not exposed to anything outside of this crate.
    RaftInternal,
    /// The `RaftNetwork` impl supplied to the Raft node.
    RaftNetwork,
    /// The `RaftStorage` impl supplied to the Raft node.
    RaftStorage,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// UpdateCurrentLeader ///////////////////////////////////////////////////////////////////////////

/// An enum describing the way the current leader property is to be updated.
pub(crate) enum UpdateCurrentLeader {
    Unknown,
    OtherNode(NodeId),
    ThisNode,
}
