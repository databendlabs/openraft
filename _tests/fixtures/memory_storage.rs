use std::{
    collections::BTreeMap,
    io::{Seek, SeekFrom, Write},
    fs::{self, File},
    path::PathBuf,
};

use actix::prelude::*;
use log::{debug, error};
use serde::{Serialize, Deserialize};
use rmp_serde as rmps;

use actix_raft::{
    AppData, AppDataResponse, AppError, NodeId,
    messages::{Entry as RaftEntry, EntrySnapshotPointer, MembershipConfig},
    storage::{
        AppendEntryToLog,
        ReplicateToLog,
        ApplyEntryToStateMachine,
        ReplicateToStateMachine,
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

type Entry = RaftEntry<MemoryStorageData>;

/// The concrete data type used by the `MemoryStorage` system.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemoryStorageData {
    pub data: Vec<u8>,
}

impl AppData for MemoryStorageData {}

/// The concrete data type used for responding from the storage engine when applying logs to the state machine.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemoryStorageResponse;

impl AppDataResponse for MemoryStorageResponse {}

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
    log: BTreeMap<u64, Entry>,
    snapshot_data: Option<CurrentSnapshotData>,
    snapshot_dir: String,
    state_machine: BTreeMap<u64, Entry>,
    snapshot_actor: Addr<SnapshotActor>,
}

impl MemoryStorage {
    /// Create a new instance.
    pub fn new(members: Vec<NodeId>, snapshot_dir: String) -> Self {
        let snapshot_dir_pathbuf = std::path::PathBuf::from(snapshot_dir.clone());
        let membership = MembershipConfig{members, non_voters: vec![], removing: vec![], is_in_joint_consensus: false};
        Self{
            hs: HardState{current_term: 0, voted_for: None, membership},
            log: Default::default(),
            snapshot_data: None, snapshot_dir,
            state_machine: Default::default(),
            snapshot_actor: SyncArbiter::start(1, move || SnapshotActor(snapshot_dir_pathbuf.clone())),
        }
    }
}

impl Actor for MemoryStorage {
    type Context = Context<Self>;

    /// Start this actor.
    fn started(&mut self, _ctx: &mut Self::Context) {}
}

impl RaftStorage<MemoryStorageData, MemoryStorageResponse, MemoryStorageError> for MemoryStorage {
    type Actor = Self;
    type Context = Context<Self>;
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

impl Handler<GetLogEntries<MemoryStorageData, MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, Vec<Entry>, MemoryStorageError>;

    fn handle(&mut self, msg: GetLogEntries<MemoryStorageData, MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        Box::new(fut::ok(self.log.range(msg.start..msg.stop).map(|e| e.1.clone()).collect()))
    }
}

impl Handler<AppendEntryToLog<MemoryStorageData, MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;

    fn handle(&mut self, msg: AppendEntryToLog<MemoryStorageData, MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        self.log.insert(msg.entry.index, (*msg.entry).clone());
        Box::new(fut::ok(()))
    }
}

impl Handler<ReplicateToLog<MemoryStorageData, MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;

    fn handle(&mut self, msg: ReplicateToLog<MemoryStorageData, MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        msg.entries.iter().for_each(|e| {
            self.log.insert(e.index, e.clone());
        });
        Box::new(fut::ok(()))
    }
}

impl Handler<ApplyEntryToStateMachine<MemoryStorageData, MemoryStorageResponse, MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, MemoryStorageResponse, MemoryStorageError>;

    fn handle(&mut self, msg: ApplyEntryToStateMachine<MemoryStorageData, MemoryStorageResponse, MemoryStorageError>, _ctx: &mut Self::Context) -> Self::Result {
        let res = if let Some(old) = self.state_machine.insert(msg.payload.index, (*msg.payload).clone()) {
            error!("Critical error. State machine entires are not allowed to be overwritten. Entry: {:?}", old);
            Err(MemoryStorageError)
        } else {
            Ok(MemoryStorageResponse)
        };
        Box::new(fut::result(res))
    }
}

impl Handler<ReplicateToStateMachine<MemoryStorageData, MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;

    fn handle(&mut self, msg: ReplicateToStateMachine<MemoryStorageData, MemoryStorageError>, _ctx: &mut Self::Context) -> Self::Result {
        let res = msg.payload.iter().try_for_each(|e| {
            if let Some(old) = self.state_machine.insert(e.index, e.clone()) {
                error!("Critical error. State machine entires are not allowed to be overwritten. Entry: {:?}", old);
                return Err(MemoryStorageError)
            }
            Ok(())
        });
        Box::new(fut::result(res))
    }
}

impl Handler<CreateSnapshot<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, CurrentSnapshotData, MemoryStorageError>;

    fn handle(&mut self, msg: CreateSnapshot<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        debug!("Creating new snapshot under '{}' through index {}.", &self.snapshot_dir, &msg.through);
        // Serialize snapshot data.
        let through = msg.through;
        let entries = self.log.range(0u64..=through).map(|(_, v)| v.clone()).collect::<Vec<_>>();
        debug!("Creating snapshot with {} entries.", entries.len());
        let (index, term) = entries.last().map(|e| (e.index, e.term)).unwrap_or((0, 0));
        let snapdata = match rmps::to_vec(&entries) {
            Ok(snapdata) => snapdata,
            Err(err) => {
                error!("Error serializing log for creating a snapshot. {}", err);
                return Box::new(fut::err(MemoryStorageError));
            }
        };

        // Create snapshot file and write snapshot data to it.
        let filename = format!("{}", msg.through);
        let filepath = std::path::PathBuf::from(self.snapshot_dir.clone()).join(filename);
        Box::new(fut::wrap_future(self.snapshot_actor.send(CreateSnapshotWithData(filepath.clone(), snapdata)))
            .map_err(|err, _, _| panic!("Error communicating with snapshot actor. {}", err))
            .and_then(|res, _, _| fut::result(res))
            // Clean up old log entries which are now part of the new snapshot.
            .and_then(move |_, act: &mut Self, _| {
                let path = filepath.to_string_lossy().to_string();
                debug!("Finished creating snapshot file at {}", &path);
                act.log = act.log.split_off(&through);
                let pointer = EntrySnapshotPointer{path};
                let entry = Entry::new_snapshot_pointer(pointer.clone(), index, term);
                act.log.insert(through, entry);

                // Cache the most recent snapshot data.
                let current_snap_data = CurrentSnapshotData{term, index, membership: act.hs.membership.clone(), pointer};
                act.snapshot_data = Some(current_snap_data.clone());

                fut::ok(current_snap_data)
            }))
    }
}

impl Handler<InstallSnapshot<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;

    fn handle(&mut self, msg: InstallSnapshot<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        let (index, term) = (msg.index, msg.term);
        Box::new(fut::wrap_future(self.snapshot_actor.send(SyncInstallSnapshot(msg)))
            .map_err(|err, _, _| panic!("Error communicating with snapshot actor. {}", err))
            .and_then(|res, _, _| fut::result(res))

            // Snapshot file has been created. Perform final steps of this algorithm.
            .and_then(move |pointer, act: &mut Self, ctx| {
                // Cache the most recent snapshot data.
                act.snapshot_data = Some(CurrentSnapshotData{index, term, membership: act.hs.membership.clone(), pointer: pointer.clone()});

                // Update target index with the new snapshot pointer.
                let entry = Entry::new_snapshot_pointer(pointer.clone(), index, term);
                act.log = act.log.split_off(&index);
                let previous = act.log.insert(index, entry);

                // If there are any logs newer than `index`, then we are done. Else, the state
                // machine should be reset, and recreated from the new snapshot.
                match &previous {
                    Some(entry) if entry.index == index && entry.term == term => {
                        fut::Either::A(fut::ok(()))
                    }
                    // There are no newer entries in the log, which means that we need to rebuild
                    // the state machine. Open the snapshot file read out its entries.
                    _ => {
                        let pathbuf = PathBuf::from(pointer.path);
                        fut::Either::B(act.rebuild_state_machine_from_snapshot(ctx, pathbuf))
                    }
                }
            }))
    }
}

impl Handler<GetCurrentSnapshot<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, Option<CurrentSnapshotData>, MemoryStorageError>;

    fn handle(&mut self, _: GetCurrentSnapshot<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        debug!("Checking for current snapshot.");
        Box::new(fut::ok(self.snapshot_data.clone()))
    }
}

impl MemoryStorage {
    /// Rebuild the state machine from the specified snapshot.
    fn rebuild_state_machine_from_snapshot(&mut self, _: &mut Context<Self>, path: std::path::PathBuf) -> impl ActorFuture<Actor=Self, Item=(), Error=MemoryStorageError> {
        // Read full contents of the snapshot file.
        fut::wrap_future(self.snapshot_actor.send(DeserializeSnapshot(path)))
            .map_err(|err, _, _| panic!("Error communicating with snapshot actor. {}", err))
            .and_then(|res, _, _| fut::result(res))
            // Rebuild state machine from the deserialized data.
            .and_then(|entries, act: &mut Self, _| {
                act.state_machine.clear();
                act.state_machine.extend(entries.into_iter().map(|e| (e.index, e)));
                fut::ok(())
            })
            .map(|_, _, _| debug!("Finished rebuilding statemachine from snapshot successfully."))
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// SnapshotActor /////////////////////////////////////////////////////////////////////////////////

/// A simple synchronous actor for interfacing with the filesystem for snapshots.
struct SnapshotActor(std::path::PathBuf);

impl Actor for SnapshotActor {
    type Context = SyncContext<Self>;
}

struct SyncInstallSnapshot(InstallSnapshot<MemoryStorageError>);

//////////////////////////////////////////////////////////////////////////////
// CreateSnapshotWithData ////////////////////////////////////////////////////

struct CreateSnapshotWithData(PathBuf, Vec<u8>);

impl Message for CreateSnapshotWithData {
    type Result = Result<(), MemoryStorageError>;
}

impl Handler<CreateSnapshotWithData> for SnapshotActor {
    type Result = Result<(), MemoryStorageError>;

    fn handle(&mut self, msg: CreateSnapshotWithData, _: &mut Self::Context) -> Self::Result {
        fs::write(msg.0.clone(), msg.1).map_err(|err| {
            error!("Error writing snapshot file. {}", err);
            MemoryStorageError
        })
    }
}

//////////////////////////////////////////////////////////////////////////////
// DeserializeSnapshot ///////////////////////////////////////////////////////

struct DeserializeSnapshot(PathBuf);

impl Message for DeserializeSnapshot {
    type Result = Result<Vec<Entry>, MemoryStorageError>;
}

impl Handler<DeserializeSnapshot> for SnapshotActor {
    type Result = Result<Vec<Entry>, MemoryStorageError>;

    fn handle(&mut self, msg: DeserializeSnapshot, _: &mut Self::Context) -> Self::Result {
        fs::read(msg.0)
            .map_err(|err| {
                error!("Error reading contents of snapshot file. {}", err);
                MemoryStorageError
            })
            // Deserialize the data of the snapshot file.
            .and_then(|snapdata| {
                rmps::from_slice::<Vec<Entry>>(snapdata.as_slice()).map_err(|err| {
                    error!("Error deserializing snapshot contents. {}", err);
                    MemoryStorageError
                })
            })
    }
}

//////////////////////////////////////////////////////////////////////////////
// SyncInstallSnapshot ///////////////////////////////////////////////////////

impl Message for SyncInstallSnapshot {
    type Result = Result<EntrySnapshotPointer, MemoryStorageError>;
}

impl Handler<SyncInstallSnapshot> for SnapshotActor {
    type Result = Result<EntrySnapshotPointer, MemoryStorageError>;

    fn handle(&mut self, msg: SyncInstallSnapshot, _: &mut Self::Context) -> Self::Result {
        let filename = format!("{}", &msg.0.index);
        let filepath = std::path::PathBuf::from(self.0.clone()).join(filename);

        // Create the new snapshot file.
        let mut snapfile = File::create(&filepath).map_err(|err| {
            error!("Error creating new snapshot file. {}", err);
            MemoryStorageError
        })?;

        let chunk_stream = msg.0.stream.map_err(|_| {
            error!("Snapshot chunk stream hit an error in the memory_storage system.");
            MemoryStorageError
        }).wait();
        let mut did_process_final_chunk = false;
        for chunk in chunk_stream {
            let chunk = chunk?;
            snapfile.seek(SeekFrom::Start(chunk.offset)).map_err(|err| {
                error!("Error seeking to file location for writing snapshot chunk. {}", err);
                MemoryStorageError
            })?;
            snapfile.write_all(&chunk.data).map_err(|err| {
                error!("Error writing snapshot chunk to snapshot file. {}", err);
                MemoryStorageError
            })?;
            if chunk.done {
                did_process_final_chunk = true;
            }
            let _ = chunk.cb.send(());
        }

        if !did_process_final_chunk {
            error!("Prematurely exiting snapshot chunk stream. Never hit final chunk.");
            Err(MemoryStorageError)
        } else {
            Ok(EntrySnapshotPointer{path: filepath.to_string_lossy().to_string()})
        }
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// Other Message Types & Handlers ////////////////////////////////////////////////////////////////
//
// NOTE WELL: these following types, just as the MemoryStorage system overall, is intended
// primarily for testing purposes. Don't build your application using this storage implementation.

/// Get the current state of the storage engine.
pub struct GetCurrentState;

impl Message for GetCurrentState {
    type Result = Result<CurrentStateData, ()>;
}

/// The current state of the storage engine.
pub struct CurrentStateData {
    pub hs: HardState,
    pub log: BTreeMap<u64, Entry>,
    pub snapshot_data: Option<CurrentSnapshotData>,
    pub snapshot_dir: String,
    pub state_machine: BTreeMap<u64, Entry>,
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
