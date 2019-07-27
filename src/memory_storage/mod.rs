use std::collections::BTreeMap;

use actix::prelude::*;
use log::{error};
use serde::{Serialize, Deserialize};

use crate::{
    AppError, NodeId,
    messages,
    storage::{
        AppendLogEntry,
        ReplicateLogEntries,
        ApplyToStateMachine,
        ApplyToStateMachinePayload,
        CreateSnapshot,
        CurrentSnapshotData,
        GetCurrentSnapshot,
        GetInitialState,
        GetLogEntries,
        HardState,
        InitialState,
        InstallSnapshot,
        RaftStorage,
        SaveHardState,
    },
};

/// The concrete error type used by the `MemoryStorage` system.
#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryStorageError;

impl std::fmt::Display for MemoryStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: give this something a bit more meaningful.
        write!(f, "MemoryStorageError")
    }
}

impl std::error::Error for MemoryStorageError {}

impl AppError for MemoryStorageError {}

/// A concrete implementation of the `RaftStorage` trait.
///
/// This is primarity for testing and demo purposes. In a real application, storing Raft's data
/// on a stable storage medium is expected.
///
/// This storage implementation structures its data as an append-only immutable log. The contents
/// of the entries given to this storage implementation are not ready or manipulated.
pub struct MemoryStorage {
    hs: HardState,
    log: BTreeMap<u64, messages::Entry>,
    snapshot_data: Option<CurrentSnapshotData>,
    snapshot_dir: String,
    state_machine: BTreeMap<u64, messages::Entry>,
}

impl RaftStorage<MemoryStorageError> for MemoryStorage {
    /// Create a new instance.
    fn new(members: Vec<NodeId>, snapshot_dir: String) -> Self {
        Self{
            hs: HardState{current_term: 0, voted_for: None, members},
            log: Default::default(),
            snapshot_data: None, snapshot_dir,
            state_machine: Default::default(),
        }
    }
}

impl Actor for MemoryStorage {
    type Context = Context<Self>;

    /// Start this actor.
    fn started(&mut self, _ctx: &mut Self::Context) {}
}

impl Handler<GetInitialState<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, InitialState, MemoryStorageError>;

    fn handle(&mut self, _: GetInitialState<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        Box::new(fut::ok(InitialState{
            last_log_index: self.log.iter().last().map(|e| *e.0).unwrap_or(0),
            last_log_term: self.log.iter().last().map(|e| e.1.term).unwrap_or(0),
            last_applied_log: self.state_machine.iter().last().map(|e| *e.0).unwrap_or(0),
            hard_state: self.hs.clone(),
        }))
    }
}

impl Handler<SaveHardState<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;

    fn handle(&mut self, msg: SaveHardState<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        self.hs = msg.hs;
        Box::new(fut::ok(()))
    }
}

impl Handler<GetLogEntries<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, Vec<messages::Entry>, MemoryStorageError>;

    fn handle(&mut self, msg: GetLogEntries<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        Box::new(fut::ok(self.log.range(msg.start..msg.stop).map(|e| e.1.clone()).collect()))
    }
}

impl Handler<AppendLogEntry<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;

    fn handle(&mut self, msg: AppendLogEntry<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        self.log.insert(msg.entry.index, (*msg.entry).clone());
        Box::new(fut::ok(()))
    }
}

impl Handler<ReplicateLogEntries<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;

    fn handle(&mut self, msg: ReplicateLogEntries<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        msg.entries.iter().for_each(|e| {
            self.log.insert(e.index, e.clone());
        });
        Box::new(fut::ok(()))
    }
}

impl Handler<ApplyToStateMachine<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;

    fn handle(&mut self, msg: ApplyToStateMachine<MemoryStorageError>, _ctx: &mut Self::Context) -> Self::Result {
        let res = match msg.payload {
            ApplyToStateMachinePayload::Multi(entries) => {
                entries.iter().try_for_each(|e| {
                    if let Some(old) = self.state_machine.insert(e.index, e.clone()) {
                        error!("Critical error. State machine entires are not allowed to be overwritten. Entry: {:?}", old);
                        return Err(MemoryStorageError)
                    }
                    Ok(())
                })
            }
            ApplyToStateMachinePayload::Single(entry) => {
                if let Some(old) = self.state_machine.insert(entry.index, (*entry).clone()) {
                    error!("Critical error. State machine entires are not allowed to be overwritten. Entry: {:?}", old);
                    Err(MemoryStorageError)
                } else {
                    Ok(())
                }
            }
        };
        Box::new(fut::result(res))
    }
}

impl Handler<CreateSnapshot<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, CurrentSnapshotData, MemoryStorageError>;

    fn handle(&mut self, _msg: CreateSnapshot<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        error!("Error received request to create snapshot, and snapshots have not been implemented yet.");
        Box::new(fut::err(MemoryStorageError))
    }
}

impl Handler<InstallSnapshot<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;

    fn handle(&mut self, _msg: InstallSnapshot<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        error!("Error received request to install snapshot, and snapshots have not been implemented yet.");
        Box::new(fut::err(MemoryStorageError))
    }
}

impl Handler<GetCurrentSnapshot<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, Option<CurrentSnapshotData>, MemoryStorageError>;

    fn handle(&mut self, _: GetCurrentSnapshot<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        error!("Error received request to get current snapshot, and snapshots have not been implemented yet.");
        Box::new(fut::ok(self.snapshot_data.clone()))
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// Other Message Types & Handlers ////////////////////////////////////////////////////////////////

/// Get the current state of the storage engine.
pub struct GetCurrentState;

impl Message for GetCurrentState {
    type Result = Result<CurrentStateData, ()>;
}

/// The current state of the storage engine.
pub struct CurrentStateData {
    pub hs: HardState,
    pub log: BTreeMap<u64, messages::Entry>,
    pub snapshot_data: Option<CurrentSnapshotData>,
    pub snapshot_dir: String,
    pub state_machine: BTreeMap<u64, messages::Entry>,
}

impl Handler<GetCurrentState> for MemoryStorage {
    type Result = Result<CurrentStateData, ()>;

    fn handle(&mut self, _: GetCurrentState, _: &mut Self::Context) -> Self::Result {
        Ok(CurrentStateData{
            hs: self.hs.clone(),
            log: self.log.clone(),
            snapshot_data: self.snapshot_data.clone(),
            snapshot_dir: self.snapshot_dir.clone(),
            state_machine: self.state_machine.clone(),
        })
    }
}
