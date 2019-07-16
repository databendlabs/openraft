use actix::prelude::*;
use serde::{Serialize, Deserialize};

use crate::{
    AppError,
    messages,
    storage::{
        AppendLogEntries,
        ApplyEntriesToStateMachine,
        CreateSnapshot,
        CurrentSnapshotData,
        GetCurrentSnapshot,
        GetInitialState,
        GetLogEntries,
        InitialState,
        InstallSnapshot,
        RaftStorage,
        SaveHardState,
    },
};

/// The concrete error type used by the `MemoryStorage` system.
#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryStorageError;

impl AppError for MemoryStorageError {}

/// A concrete implementation of the `RaftStorage` trait.
///
/// This is primarity for testing and demo purposes. In a real application, storing Raft's data
/// on a stable storage medium is expected.
pub struct MemoryStorage {
    #[allow(dead_code)] // TODO: remove this after impl.
    snapshot_dir: String,
}

impl RaftStorage<MemoryStorageError> for MemoryStorage {
    fn new(snapshot_dir: String) -> Self {
        Self{snapshot_dir}
    }
}

impl Actor for MemoryStorage {
    type Context = Context<Self>;

    /// Start this actor.
    fn started(&mut self, _ctx: &mut Self::Context) {
    }
}

impl Handler<GetInitialState<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, InitialState, MemoryStorageError>;
    fn handle(&mut self, _msg: GetInitialState<MemoryStorageError>, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::err(MemoryStorageError))
    }
}

impl Handler<SaveHardState<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;
    fn handle(&mut self, _msg: SaveHardState<MemoryStorageError>, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::err(MemoryStorageError))
    }
}

impl Handler<GetLogEntries<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, Vec<messages::Entry>, MemoryStorageError>;
    fn handle(&mut self, _msg: GetLogEntries<MemoryStorageError>, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::err(MemoryStorageError))
    }
}

impl Handler<AppendLogEntries<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;
    fn handle(&mut self, _msg: AppendLogEntries<MemoryStorageError>, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::err(MemoryStorageError))
    }
}

impl Handler<ApplyEntriesToStateMachine<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;
    fn handle(&mut self, _msg: ApplyEntriesToStateMachine<MemoryStorageError>, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::err(MemoryStorageError))
    }
}

impl Handler<CreateSnapshot<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, CurrentSnapshotData, MemoryStorageError>;
    fn handle(&mut self, _msg: CreateSnapshot<MemoryStorageError>, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::err(MemoryStorageError))
    }
}

impl Handler<InstallSnapshot<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;
    fn handle(&mut self, _msg: InstallSnapshot<MemoryStorageError>, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::err(MemoryStorageError))
    }
}

impl Handler<GetCurrentSnapshot<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, Option<CurrentSnapshotData>, MemoryStorageError>;
    fn handle(&mut self, _msg: GetCurrentSnapshot<MemoryStorageError>, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::err(MemoryStorageError))
    }
}
