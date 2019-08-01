use std::collections::BTreeMap;

use actix::prelude::*;
use log::{debug, error};
use rmp_serde as rmps;
use serde::{Serialize, Deserialize};
use tokio::{self, io::{AsyncWrite}};

use crate::{
    AppError, NodeId,
    messages::{Entry, EntrySnapshotPointer},
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
    log: BTreeMap<u64, Entry>,
    snapshot_data: Option<CurrentSnapshotData>,
    snapshot_dir: String,
    state_machine: BTreeMap<u64, Entry>,
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
    type Result = ResponseActFuture<Self, Vec<Entry>, MemoryStorageError>;

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

    fn handle(&mut self, msg: CreateSnapshot<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        debug!("Creating new snapshot.");
        // Serialize snapshot data.
        let through = msg.through;
        let entries = &self.log.range(0u64..=through).map(|(_, v)| v).collect::<Vec<_>>();
        let (index, term) = entries.last().map(|e| (e.index, e.term)).unwrap_or((0, 0));
        let snapdata = match rmps::to_vec(entries) {
            Ok(snapdata) => snapdata,
            Err(err) => {
                error!("Error serializing log for creating a snapshot. {}", err);
                return Box::new(fut::err(MemoryStorageError));
            }
        };

        // Create snapshot file and write snapshot data to it.
        let filename = format!("{}", msg.through);
        let filepath = std::path::PathBuf::from(self.snapshot_dir.clone()).join(filename);
        Box::new(fut::wrap_future(tokio::fs::write(filepath.clone(), snapdata)
            .map(|_| ())
            .map_err(|err| {
                error!("Error writing snapshot file. {}", err);
                MemoryStorageError
            }))
            // Clean up old log entries which are now part of the new snapshot.
            .and_then(move |_, act: &mut Self, _| {
                act.log = act.log.split_off(&through);
                let pointer = EntrySnapshotPointer{path: filepath.to_string_lossy().to_string()};
                let entry = Entry::new_snapshot_pointer(pointer.clone(), index, term);
                act.log.insert(through, entry);
                fut::ok(CurrentSnapshotData{term, index, config: act.hs.members.clone(), pointer})
            }))
    }
}

impl Handler<InstallSnapshot<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, (), MemoryStorageError>;

    fn handle(&mut self, msg: InstallSnapshot<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        debug!("Streaming in snapshot to install new snapshot.");
        let (index, term, chunk_stream) = (msg.index, msg.term, msg.stream);
        let filename = format!("{}", &index);
        let filepath = std::path::PathBuf::from(self.snapshot_dir.clone()).join(filename);
        let filepath0 = filepath.clone();
        let task = fut::wrap_future(tokio::fs::File::create(filepath.clone())
            .map_err(|err| {
                error!("Error creating new snapshot file. {}", err);
                MemoryStorageError
            })
            // File is open, start streaming in the chunks and write them to the file as needed.
            .and_then(move |_| {
                chunk_stream
                    .map_err(|_| MemoryStorageError)
                    .for_each(move |chunk| {
                        let offset = chunk.offset;
                        tokio::fs::OpenOptions::new().append(true).open(filepath.clone()).map_err(|err| {
                            error!("Error opening snapshot file. {}", err);
                            MemoryStorageError
                        })
                        .and_then(move |file| {
                            file.seek(std::io::SeekFrom::Start(offset))
                                .map_err(|err| {
                                    error!("Error seeking to file location for writing snapshot chunk. {}", err);
                                    MemoryStorageError
                                })
                                .and_then(|(mut file, _)| {
                                    futures::future::poll_fn(move || file.poll_write(&chunk.data))
                                        .map(|_| ())
                                        .map_err(|err| {
                                            error!("Error writing snapshot chunk bytes to snapshot file. {}", err);
                                            MemoryStorageError
                                        })
                                })
                        })
                    })
            }))
            // Snapshot file has been created. Perform final steps of this algorithm.
            .and_then(move |_, act: &mut Self, ctx| {
                let pathbuf = filepath0.clone();
                let path = pathbuf.clone().to_string_lossy().to_string();

                // Cache the most recent snapshot data.
                let pointer = EntrySnapshotPointer{path: path.clone()};
                act.snapshot_data = Some(CurrentSnapshotData{index, term, config: act.hs.members.clone(), pointer: pointer.clone()});

                // Update target index with the new snapshot pointer.
                let entry = Entry::new_snapshot_pointer(pointer, index, term);
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
                        fut::Either::B(act.rebuild_state_machine_from_snapshot(ctx, pathbuf))
                    }
                }
            });
        Box::new(task)
    }
}

impl Handler<GetCurrentSnapshot<MemoryStorageError>> for MemoryStorage {
    type Result = ResponseActFuture<Self, Option<CurrentSnapshotData>, MemoryStorageError>;

    fn handle(&mut self, _: GetCurrentSnapshot<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        Box::new(fut::ok(self.snapshot_data.clone()))
    }
}

impl MemoryStorage {
    /// Rebuild the state machine from the specified snapshot.
    fn rebuild_state_machine_from_snapshot(&mut self, _: &mut Context<Self>, path: std::path::PathBuf) -> impl ActorFuture<Actor=Self, Item=(), Error=MemoryStorageError> {
        // Read full contents of the snapshot file.
        fut::wrap_future(tokio::fs::read(path)
            .map_err(|err| {
                error!("Error reading contents of snapshot file. {}", err);
                MemoryStorageError
            }))
            // Deserialize the data of the snapshot file.
            .and_then(|snapdata, _, _| {
                let entries: Result<Vec<Entry>, MemoryStorageError> = rmps::from_slice(snapdata.as_slice())
                    .map_err(|err| {
                        error!("Error deserializing snapshot contents. {}", err);
                        MemoryStorageError
                    });
                fut::result(entries)
            })
            // Rebuild state machine from the deserialized data.
            .and_then(|entries, act: &mut Self, _| {
                act.state_machine.clear();
                act.state_machine.extend(entries.into_iter().map(|e| (e.index, e)));
                fut::ok(())
            })
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
