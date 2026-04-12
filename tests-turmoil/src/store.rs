//! In-memory storage for turmoil tests.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt::Debug;
use std::io;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::Mutex;

use futures::Stream;
use openraft::EntryPayload;
use openraft::OptionalSend;
use openraft::storage::EntryResponder;
use openraft::storage::IOFlushed;
use openraft::storage::LogState;
use openraft::storage::RaftLogReader;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftSnapshotBuilder;
use openraft::storage::RaftStateMachine;
use serde::Deserialize;
use serde::Serialize;

use crate::typ::*;

/// State machine data.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateMachineData {
    pub last_applied: Option<LogId>,
    pub last_membership: StoredMembership,
    /// Key-value store.
    pub data: HashMap<String, String>,
}

/// In-memory log store.
pub struct LogStore {
    vote: Mutex<Option<Vote>>,
    committed: Mutex<Option<LogId>>,
    last_purged: Mutex<Option<LogId>>,
    log: Mutex<BTreeMap<u64, Entry>>,
}

impl Default for LogStore {
    fn default() -> Self {
        Self::new()
    }
}

impl LogStore {
    pub fn new() -> Self {
        Self {
            vote: Mutex::new(None),
            committed: Mutex::new(None),
            last_purged: Mutex::new(None),
            log: Mutex::new(BTreeMap::new()),
        }
    }
}

impl RaftLogReader<TypeConfig> for Arc<LogStore> {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry>, io::Error> {
        let log = self.log.lock().unwrap();
        let entries: Vec<Entry> = log.range(range).map(|(_, e)| e.clone()).collect();
        Ok(entries)
    }

    async fn read_vote(&mut self) -> Result<Option<Vote>, io::Error> {
        Ok(*self.vote.lock().unwrap())
    }

    async fn limited_get_log_entries(&mut self, start: u64, end: u64) -> Result<Vec<Entry>, io::Error> {
        self.try_get_log_entries(start..end).await
    }
}

impl RaftLogStorage<TypeConfig> for Arc<LogStore> {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, io::Error> {
        let log = self.log.lock().unwrap();
        let last = log.iter().next_back().map(|(_, e)| e.log_id);
        let last_purged = *self.last_purged.lock().unwrap();

        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id: last.or(last_purged),
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &Vote) -> Result<(), io::Error> {
        *self.vote.lock().unwrap() = Some(*vote);
        Ok(())
    }

    async fn save_committed(&mut self, committed: Option<LogId>) -> Result<(), io::Error> {
        *self.committed.lock().unwrap() = committed;
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogId>, io::Error> {
        Ok(*self.committed.lock().unwrap())
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<TypeConfig>) -> Result<(), io::Error>
    where I: IntoIterator<Item = Entry> + OptionalSend {
        let mut log = self.log.lock().unwrap();
        for entry in entries {
            log.insert(entry.log_id.index, entry);
        }
        callback.io_completed(Ok(()));
        Ok(())
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogId>) -> Result<(), io::Error> {
        let start_index = match last_log_id {
            Some(log_id) => log_id.index + 1,
            None => 0,
        };
        self.log.lock().unwrap().split_off(&start_index);
        Ok(())
    }

    async fn purge(&mut self, log_id: LogId) -> Result<(), io::Error> {
        *self.last_purged.lock().unwrap() = Some(log_id);
        let mut log = self.log.lock().unwrap();
        *log = log.split_off(&(log_id.index + 1));
        Ok(())
    }
}

/// In-memory state machine.
pub struct StateMachine {
    data: Mutex<StateMachineData>,
    snapshot: Mutex<Option<(SnapshotMeta, Vec<u8>)>>,
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl StateMachine {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(StateMachineData::default()),
            snapshot: Mutex::new(None),
        }
    }

    /// Get a clone of the current state machine data.
    pub fn get_data(&self) -> StateMachineData {
        self.data.lock().unwrap().clone()
    }
}

impl RaftSnapshotBuilder<TypeConfig> for Arc<StateMachine> {
    async fn build_snapshot(&mut self) -> Result<Snapshot, io::Error> {
        let data = self.data.lock().unwrap();
        let snapshot_data = serde_json::to_vec(&*data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let snapshot_idx = {
            let snap = self.snapshot.lock().unwrap();
            snap.as_ref().map(|(m, _)| m.snapshot_id.parse::<u64>().unwrap_or(0) + 1).unwrap_or(1)
        };

        let meta = SnapshotMeta {
            last_log_id: data.last_applied,
            last_membership: data.last_membership.clone(),
            snapshot_id: snapshot_idx.to_string(),
        };

        *self.snapshot.lock().unwrap() = Some((meta.clone(), snapshot_data.clone()));

        Ok(Snapshot {
            meta,
            snapshot: Cursor::new(snapshot_data),
        })
    }
}

impl RaftStateMachine<TypeConfig> for Arc<StateMachine> {
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogId>, StoredMembership), io::Error> {
        let data = self.data.lock().unwrap();
        Ok((data.last_applied, data.last_membership.clone()))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend {
        use futures::TryStreamExt;

        // Collect all entries first to avoid holding lock across await
        let mut collected = vec![];
        while let Some(entry_responder) = entries.try_next().await? {
            collected.push(entry_responder);
        }

        tracing::info!("Applying {} entries to state machine", collected.len());

        // Now apply all entries while holding the lock
        let mut data = self.data.lock().unwrap();

        for (entry, responder) in collected {
            data.last_applied = Some(entry.log_id);

            let response = match entry.payload {
                EntryPayload::Blank => Response { value: None },
                EntryPayload::Normal(ref req) => {
                    let prev = data.data.insert(req.key.clone(), req.value.clone());
                    Response { value: prev }
                }
                EntryPayload::Membership(ref mem) => {
                    data.last_membership = StoredMembership::new(Some(entry.log_id), mem.clone());
                    Response { value: None }
                }
            };

            if let Some(responder) = responder {
                responder.send(response);
            }
        }

        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn begin_receiving_snapshot(&mut self) -> Result<SnapshotData, io::Error> {
        Ok(Cursor::new(Vec::new()))
    }

    async fn install_snapshot(&mut self, meta: &SnapshotMeta, snapshot: SnapshotData) -> Result<(), io::Error> {
        let new_data: StateMachineData =
            serde_json::from_slice(snapshot.get_ref()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        *self.data.lock().unwrap() = new_data;
        *self.snapshot.lock().unwrap() = Some((meta.clone(), snapshot.into_inner()));

        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot>, io::Error> {
        let snap = self.snapshot.lock().unwrap();
        match &*snap {
            Some((meta, data)) => Ok(Some(Snapshot {
                meta: meta.clone(),
                snapshot: Cursor::new(data.clone()),
            })),
            None => Ok(None),
        }
    }
}

/// Create a new log store and state machine pair.
pub fn new_store() -> (Arc<LogStore>, Arc<StateMachine>) {
    (Arc::new(LogStore::new()), Arc::new(StateMachine::new()))
}
