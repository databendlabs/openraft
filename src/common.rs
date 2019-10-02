//! Common types and functionality used by the Raft actor.

use std::sync::Arc;

use futures::sync::oneshot;

use crate::{
    AppData, AppError, NodeId,
    messages::{
        ClientError, ClientPayload, ClientPayloadResponse,
        Entry, ResponseMode,
    },
};

pub(crate) const CLIENT_RPC_RX_ERR: &str = "Client RPC channel receiver was unexpectedly closed.";
pub(crate) const CLIENT_RPC_TX_ERR: &str = "Client RPC channel sender was unexpectedly closed.";

//////////////////////////////////////////////////////////////////////////////////////////////////
// ApplyLogsTask /////////////////////////////////////////////////////////////////////////////////

/// A task declaring some set of logs which need to be applied to the state machine.
pub(crate) enum ApplyLogsTask<D: AppData, E: AppError> {
    /// Check for & apply any logs which have been committed but which have not yet been applied.
    Outstanding,
    /// Logs which need to be applied are supplied for immediate use.
    Entry {
        /// The payload of logs to be applied.
        entry: Arc<Entry<D>>,
        /// The optional response channel for when this task is complete.
        chan: Option<oneshot::Sender<Result<ClientPayloadResponse, ClientError<D, E>>>>,
    },
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ClientPayloadWithChan /////////////////////////////////////////////////////////////////////////

pub(crate) struct ClientPayloadWithChan<D: AppData, E: AppError> {
    pub tx: oneshot::Sender<Result<ClientPayloadResponse, ClientError<D, E>>>,
    pub rpc: ClientPayload<D, E>,
}

impl<D: AppData, E: AppError> ClientPayloadWithChan<D, E> {
    /// Upgrade a client payload with an assigned index & term.
    ///
    /// - `index`: the index to assign to this payload.
    /// - `term`: the term to assign to this payload.
    pub(crate) fn upgrade(self, index: u64, term: u64) -> ClientPayloadWithIndex<D, E> {
        ClientPayloadWithIndex::new(self, index, term)
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ClientPayloadWithIndex /////////////////////////////////////////////////////////////////////////

/// A client payload which has made its way into the processing pipeline.
pub(crate) struct ClientPayloadWithIndex<D: AppData, E: AppError> {
    /// The channel of the original client request.
    pub tx: oneshot::Sender<Result<ClientPayloadResponse, ClientError<D, E>>>,
    /// The entry of the original request with an assigned index & term, ready for storage.
    entry: Arc<Entry<D>>,
    /// The response mode of the original client request.
    pub response_mode: ResponseMode,
    /// The assigned log index of this payload.
    pub index: u64,
    /// The term associated with this payload.
    pub term: u64,
}

impl<D: AppData, E: AppError> ClientPayloadWithIndex<D, E> {
    /// Create a new instance.
    pub(self) fn new(payload: ClientPayloadWithChan<D, E>, index: u64, term: u64) -> Self {
        let entry = Arc::new(Entry{index: index, term: term, payload: payload.rpc.entry.clone()});
        Self{tx: payload.tx, entry, response_mode: payload.rpc.response_mode, index, term}
    }

    /// Downgrade the payload, typically for forwarding purposes.
    pub(crate) fn downgrade(self) -> ClientPayloadWithChan<D, E> {
        let entry = match Arc::try_unwrap(self.entry) {
            Ok(entry) => entry.payload,
            Err(arc) => arc.payload.clone(),
        };
        ClientPayloadWithChan{tx: self.tx, rpc: ClientPayload::new_base(entry, self.response_mode)}
    }

    /// Get a reference to the entry encapsulated by this payload.
    pub(crate) fn entry(&self) -> Arc<Entry<D>> {
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
