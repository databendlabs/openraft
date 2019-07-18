//! Common types and functionality used by the Raft actor.

use futures::sync::{mpsc, oneshot};

use crate::{
    NodeId, AppError,
    messages::{ClientError, ClientPayload, ClientPayloadResponse},
};

//////////////////////////////////////////////////////////////////////////////////////////////////
// ClientPayloadWithTx ///////////////////////////////////////////////////////////////////////////

pub(crate) struct ClientPayloadWithTx<E: AppError> {
    pub tx: oneshot::Sender<Result<ClientPayloadResponse, ClientError<E>>>,
    pub rpc: ClientPayload<E>,
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

//////////////////////////////////////////////////////////////////////////////////////////////////
// AwaitingCommitted /////////////////////////////////////////////////////////////////////////////

/// A struct encapsulating an RPC which is awaiting to be committed.
pub(crate) struct AwaitingCommitted<E: AppError> {
    /// The index which needs to be comitted for this value to resolve.
    pub index: u64,
    /// The buffered RPC.
    pub rpc: ClientPayloadWithTx<E>,
    /// The chan to be used for resolution once the RPC's index has been comitted.
    pub chan: oneshot::Sender<ClientPayloadWithTx<E>>,
}

