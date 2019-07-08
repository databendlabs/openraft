use actix::prelude::*;
use failure::Fail;

use crate::{
    error::StorageError,
    proto,
    storage::{
        AppendLogEntries,
        AppendLogEntriesData,
        ApplyEntriesToStateMachine,
        ApplyEntriesToStateMachineData,
        CreateSnapshot,
        GetCurrentSnapshot,
        GetInitialState,
        GetLogEntries,
        // HardState,
        InitialState,
        InstallSnapshot,
        // InstallSnapshotChunk,
        RaftStorage,
        SaveHardState,
    },
};

/// The concrete error type used by the `MemoryStorage` system.
#[derive(Debug, Fail)]
#[fail(display="Error from memory storage.")]
pub struct MemoryStorageError;

/// A concrete implementation of the `RaftStorage` trait.
///
/// This is primarity for testing and demo purposes. In a real application, storing Raft's data
/// on a stable storage medium is expected.
pub struct MemoryStorage; // TODO: finish this up.

impl RaftStorage for MemoryStorage {}

impl Actor for MemoryStorage {
    type Context = Context<Self>;

    /// Start this actor.
    fn started(&mut self, ctx: &mut Self::Context) {
    }
}

impl Handler<GetInitialState> for MemoryStorage {
    type Result = ResponseActFuture<Self, InitialState, StorageError>;
    fn handle(&mut self, msg: GetInitialState, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::FutureResult::from(Err(StorageError(Box::new(MemoryStorageError)))))
    }
}

impl Handler<SaveHardState> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), StorageError>;
    fn handle(&mut self, msg: SaveHardState, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::FutureResult::from(Err(StorageError(Box::new(MemoryStorageError)))))
    }
}

impl Handler<GetLogEntries> for MemoryStorage {
    type Result = ResponseActFuture<Self, Vec<proto::Entry>, StorageError>;
    fn handle(&mut self, msg: GetLogEntries, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::FutureResult::from(Err(StorageError(Box::new(MemoryStorageError)))))
    }
}

impl Handler<AppendLogEntries> for MemoryStorage {
    type Result = ResponseActFuture<Self, AppendLogEntriesData, StorageError>;
    fn handle(&mut self, msg: AppendLogEntries, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::FutureResult::from(Err(StorageError(Box::new(MemoryStorageError)))))
    }
}

impl Handler<ApplyEntriesToStateMachine> for MemoryStorage {
    type Result = ResponseActFuture<Self, ApplyEntriesToStateMachineData, StorageError>;
    fn handle(&mut self, msg: ApplyEntriesToStateMachine, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::FutureResult::from(Err(StorageError(Box::new(MemoryStorageError)))))
    }
}

impl Handler<CreateSnapshot> for MemoryStorage {
    type Result = ResponseActFuture<Self, proto::Entry, StorageError>;
    fn handle(&mut self, msg: CreateSnapshot, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::FutureResult::from(Err(StorageError(Box::new(MemoryStorageError)))))
    }
}

impl Handler<InstallSnapshot> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), StorageError>;
    fn handle(&mut self, msg: InstallSnapshot, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::FutureResult::from(Err(StorageError(Box::new(MemoryStorageError)))))
    }
}

impl Handler<GetCurrentSnapshot> for MemoryStorage {
    type Result = ResponseActFuture<Self, Option<String>, StorageError>;
    fn handle(&mut self, msg: GetCurrentSnapshot, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::FutureResult::from(Err(StorageError(Box::new(MemoryStorageError)))))
    }
}
