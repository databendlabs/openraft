//! Common types and functionality used by the Raft actor.

use std::sync::Arc;

use futures::sync::{mpsc, oneshot};

use crate::{
    NodeId, AppError,
    messages::{
        ClientError, ClientPayload, ClientPayloadResponse,
        Entry, EntryType, ResponseMode,
    },
};

pub(crate) const CLIENT_RPC_CHAN_ERR: &str = "Client RPC channel was unexpectedly closed.";

//////////////////////////////////////////////////////////////////////////////////////////////////
// ApplyLogsTask /////////////////////////////////////////////////////////////////////////////////

/// A task declaring some set of logs which need to be applied to the state machine.
pub(crate) enum ApplyLogsTask<E: AppError> {
    /// Check for & apply any logs which have been committed but which have not yet been applied.
    Outstanding,
    /// Logs which need to be applied are supplied for immediate use.
    Entries{
        /// The payload of logs to be applied.
        entries: Arc<Vec<Entry>>,
        /// The response channel for the task.
        ///
        /// This channel expects a vector of the indices of all indexes which where applied to the
        /// state machine.
        res: Option<oneshot::Sender<Result<ClientPayloadResponse, ClientError<E>>>>,
    },
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ClientPayloadWithTx ///////////////////////////////////////////////////////////////////////////

pub(crate) struct ClientPayloadWithTx<E: AppError> {
    pub tx: oneshot::Sender<Result<ClientPayloadResponse, ClientError<E>>>,
    pub rpc: ClientPayload<E>,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ClientPayloadUpgraded /////////////////////////////////////////////////////////////////////////

/// A client payload which has made its way into the processing pipeline.
///
/// The entries of this model have been raised to `Entry` from `NormalEntry` with assigned
/// indices. In order to gurantee that these entries can be safely downgraded again to
/// `NormalEntry` instances, an accessor is provided so that the inner data could
/// not be safely mutated.
pub(crate) struct ClientPayloadUpgraded<E: AppError> {
    /// The channel of the original client request.
    pub tx: oneshot::Sender<Result<ClientPayloadResponse, ClientError<E>>>,
    /// The entries of the original client request, wrapped in an Arc.
    entries: Arc<Vec<Entry>>,
    /// The log index of the last element in the payload.
    pub last_index: u64,
    /// The response mode of the original client request.
    pub response_mode: ResponseMode,
}

impl<E: AppError> ClientPayloadUpgraded<E> {
    /// Get a Arc reference to the entries of this payload.
    pub(crate) fn entries(&self) -> Arc<Vec<Entry>> {
        self.entries.clone()
    }

    /// Upgrade a client payload.
    ///
    /// NOTE WELL: the value of `offset` should be the value of the Raft's `last_log_index`, as
    /// this is used to assign an index value for each entry in this payload. The value is
    /// incremented by `1` before its first use.
    pub(crate) fn upgrade(payload: ClientPayloadWithTx<E>, mut last_log_index: u64, term: u64) -> Self {
        let entries: Arc<Vec<_>> = Arc::new(payload.rpc.entries.into_iter().map(|data| {
            last_log_index += 1;
            Entry{
                index: last_log_index,
                term,
                entry_type: EntryType::Normal(data),
            }
        }).collect());
        Self{tx: payload.tx, entries, last_index: last_log_index, response_mode: payload.rpc.response_mode}
    }

    /// Downgrade the payload, typically for forwarding purposes.
    pub(crate) fn downgrade(self) -> ClientPayloadWithTx<E> {
        let entries = self.entries.iter().cloned().filter_map(|elem| match elem.entry_type {
            EntryType::Normal(inner) => Some(inner),
            _ => None,
        }).collect();
        ClientPayloadWithTx{
            tx: self.tx,
            rpc: ClientPayload::new(entries, self.response_mode),
        }
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
pub(crate) enum UpdateCurrentLeader<E: AppError> {
    Unknown,
    OtherNode(NodeId),
    ThisNode(mpsc::UnboundedSender<ClientPayloadWithTx<E>>),
}
