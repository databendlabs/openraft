//! This rocks-db backed storage implement the v2 storage API: [`RaftLogStorage`] and
//! [`RaftStateMachine`] traits. Its state machine is pure in-memory store with persisted
//! snapshot. In other words, `applying` a log entry does not flush data to disk at once.
//! These data will be flushed to disk when a snapshot is created.
#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]

#[cfg(test)] mod test;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::path::Path;
use std::sync::Arc;

use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use byteorder::WriteBytesExt;
use openraft::alias::SnapshotDataOf;
use openraft::storage::LogFlushed;
use openraft::storage::LogState;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftStateMachine;
use openraft::storage::Snapshot;
use openraft::AnyError;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::ErrorVerb;
use openraft::LogId;
use openraft::OptionalSend;
use openraft::RaftLogId;
use openraft::RaftLogReader;
use openraft::RaftSnapshotBuilder;
use openraft::SnapshotMeta;
use openraft::StorageError;
use openraft::StorageIOError;
use openraft::StoredMembership;
use openraft::Vote;
use rand::Rng;
use rocksdb::ColumnFamily;
use rocksdb::ColumnFamilyDescriptor;
use rocksdb::Direction;
use rocksdb::Options;
use rocksdb::DB;
use serde::Deserialize;
use serde::Serialize;

pub type RocksNodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration.
    pub TypeConfig:
        D = RocksRequest,
        R = RocksResponse,
);

/**
 * Here you will set the types of request that will interact with the raft nodes.
 * For example the `Set` will be used to write data (key and value) to the raft database.
 * The `AddNode` will append a new node to the current existing shared list of nodes.
 * You will want to add any request that can write data in all nodes here.
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum RocksRequest {
    Set { key: String, value: String },
}

/**
 * Here you will defined what type of answer you expect from reading the data of a node.
 * In this example it will return a optional value from a given key in
 * the `RocksRequest.Set`.
 *
 * TODO: Should we explain how to create multiple `AppDataResponse`?
 *
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RocksResponse {
    pub value: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RocksSnapshot {
    pub meta: SnapshotMeta<TypeConfig>,

    /// The data of the state machine at the time of this snapshot.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
#[derive(Default)]
#[derive(Serialize, Deserialize)]
pub struct StateMachine {
    pub last_applied_log: Option<LogId<RocksNodeId>>,

    pub last_membership: StoredMembership<TypeConfig>,

    /// Application data.
    pub data: BTreeMap<String, String>,
}

// TODO: restore state from snapshot
/// State machine in this implementation is a pure in-memory store.
/// It depends on the latest snapshot to restore the state when restarted.
#[derive(Debug, Clone)]
pub struct RocksStateMachine {
    db: Arc<DB>,
    sm: StateMachine,
}

impl RocksStateMachine {
    async fn new(db: Arc<DB>) -> RocksStateMachine {
        let mut state_machine = Self {
            db,
            sm: Default::default(),
        };
        let snapshot = state_machine.get_current_snapshot().await.unwrap();

        // Restore previous state from snapshot
        if let Some(s) = snapshot {
            let prev: StateMachine = serde_json::from_slice(s.snapshot.get_ref()).unwrap();
            state_machine.sm = prev;
        }

        state_machine
    }
}

#[derive(Debug, Clone)]
pub struct RocksLogStore {
    db: Arc<DB>,
}

type StorageResult<T> = Result<T, StorageError<RocksNodeId>>;

/// converts an id to a byte vector for storing in the database.
/// Note that we're using big endian encoding to ensure correct sorting of keys
fn id_to_bin(id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8);
    buf.write_u64::<BigEndian>(id).unwrap();
    buf
}

fn bin_to_id(buf: &[u8]) -> u64 {
    (&buf[0..8]).read_u64::<BigEndian>().unwrap()
}

/// Meta data of a raft-store.
///
/// In raft, except logs and state machine, the store also has to store several piece of metadata.
/// This sub mod defines the key-value pairs of these metadata.
mod meta {
    use openraft::ErrorSubject;
    use openraft::LogId;

    use crate::RocksNodeId;

    /// Defines metadata key and value
    pub(crate) trait StoreMeta {
        /// The key used to store in rocksdb
        const KEY: &'static str;

        /// The type of the value to store
        type Value: serde::Serialize + serde::de::DeserializeOwned;

        /// The subject this meta belongs to, and will be embedded into the returned storage error.
        fn subject(v: Option<&Self::Value>) -> ErrorSubject<RocksNodeId>;
    }

    pub(crate) struct LastPurged {}
    pub(crate) struct Vote {}

    impl StoreMeta for LastPurged {
        const KEY: &'static str = "last_purged_log_id";
        type Value = LogId<u64>;

        fn subject(_v: Option<&Self::Value>) -> ErrorSubject<RocksNodeId> {
            ErrorSubject::Store
        }
    }
    impl StoreMeta for Vote {
        const KEY: &'static str = "vote";
        type Value = openraft::Vote<RocksNodeId>;

        fn subject(_v: Option<&Self::Value>) -> ErrorSubject<RocksNodeId> {
            ErrorSubject::Vote
        }
    }
}

impl RocksLogStore {
    fn cf_meta(&self) -> &ColumnFamily {
        self.db.cf_handle("meta").unwrap()
    }

    fn cf_logs(&self) -> &ColumnFamily {
        self.db.cf_handle("logs").unwrap()
    }

    /// Get a store metadata.
    ///
    /// It returns `None` if the store does not have such a metadata stored.
    fn get_meta<M: meta::StoreMeta>(&self) -> Result<Option<M::Value>, StorageError<RocksNodeId>> {
        let v = self
            .db
            .get_cf(self.cf_meta(), M::KEY)
            .map_err(|e| StorageIOError::new(M::subject(None), ErrorVerb::Read, AnyError::new(&e)))?;

        let t = match v {
            None => None,
            Some(bytes) => Some(
                serde_json::from_slice(&bytes)
                    .map_err(|e| StorageIOError::new(M::subject(None), ErrorVerb::Read, AnyError::new(&e)))?,
            ),
        };
        Ok(t)
    }

    /// Save a store metadata.
    fn put_meta<M: meta::StoreMeta>(&self, value: &M::Value) -> Result<(), StorageError<RocksNodeId>> {
        let json_value = serde_json::to_vec(value)
            .map_err(|e| StorageIOError::new(M::subject(Some(value)), ErrorVerb::Write, AnyError::new(&e)))?;

        self.db
            .put_cf(self.cf_meta(), M::KEY, json_value)
            .map_err(|e| StorageIOError::new(M::subject(Some(value)), ErrorVerb::Write, AnyError::new(&e)))?;

        Ok(())
    }
}

impl RaftLogReader<TypeConfig> for RocksLogStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> StorageResult<Vec<Entry<TypeConfig>>> {
        let start = match range.start_bound() {
            std::ops::Bound::Included(x) => id_to_bin(*x),
            std::ops::Bound::Excluded(x) => id_to_bin(*x + 1),
            std::ops::Bound::Unbounded => id_to_bin(0),
        };

        let mut res = Vec::new();

        let it = self.db.iterator_cf(self.cf_logs(), rocksdb::IteratorMode::From(&start, Direction::Forward));
        for item_res in it {
            let (id, val) = item_res.map_err(read_logs_err)?;

            let id = bin_to_id(&id);
            if !range.contains(&id) {
                break;
            }

            let entry: Entry<_> = serde_json::from_slice(&val).map_err(read_logs_err)?;

            assert_eq!(id, entry.log_id.index);

            res.push(entry);
        }
        Ok(res)
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<RocksNodeId>>, StorageError<RocksNodeId>> {
        self.get_meta::<meta::Vote>()
    }
}

impl RaftSnapshotBuilder<TypeConfig> for RocksStateMachine {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<RocksNodeId>> {
        // Serialize the data of the state machine.
        let data = serde_json::to_vec(&self.sm).map_err(|e| StorageIOError::read_state_machine(&e))?;

        let last_applied_log = self.sm.last_applied_log;
        let last_membership = self.sm.last_membership.clone();

        // Generate a random snapshot index.
        let snapshot_idx: u64 = rand::thread_rng().gen_range(0..1000);

        let snapshot_id = if let Some(last) = last_applied_log {
            format!("{}-{}-{}", last.leader_id, last.index, snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        };

        let meta = SnapshotMeta {
            last_log_id: last_applied_log,
            last_membership,
            snapshot_id,
        };

        let snapshot = RocksSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        let serialized_snapshot = serde_json::to_vec(&snapshot)
            .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), AnyError::new(&e)))?;

        self.db
            .put_cf(self.db.cf_handle("sm_meta").unwrap(), "snapshot", serialized_snapshot)
            .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), AnyError::new(&e)))?;

        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

impl RaftLogStorage<TypeConfig> for RocksLogStore {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> StorageResult<LogState<TypeConfig>> {
        let last = self.db.iterator_cf(self.cf_logs(), rocksdb::IteratorMode::End).next();

        let last_log_id = match last {
            None => None,
            Some(res) => {
                let (_log_index, entry_bytes) = res.map_err(read_logs_err)?;
                let ent = serde_json::from_slice::<Entry<TypeConfig>>(&entry_bytes).map_err(read_logs_err)?;
                Some(ent.log_id)
            }
        };

        let last_purged_log_id = self.get_meta::<meta::LastPurged>()?;

        let last_log_id = match last_log_id {
            None => last_purged_log_id,
            Some(x) => Some(x),
        };

        Ok(LogState {
            last_purged_log_id,
            last_log_id,
        })
    }

    async fn save_vote(
        &mut self, vote: &Vote<RocksNodeId>, callback: LogFlushed<TypeConfig>
    ) -> Result<(), StorageError<RocksNodeId>> {
        self.put_meta::<meta::Vote>(vote)?;
        self.db.flush_wal(true).map_err(|e| StorageIOError::write_vote(&e))?;
        callback.log_io_completed(Ok(()));
        Ok(())
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<TypeConfig>,
    ) -> Result<(), StorageError<RocksNodeId>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + Send,
    {
        for entry in entries {
            let id = id_to_bin(entry.log_id.index);
            assert_eq!(bin_to_id(&id), entry.log_id.index);
            self.db
                .put_cf(
                    self.cf_logs(),
                    id,
                    serde_json::to_vec(&entry).map_err(|e| StorageIOError::write_logs(&e))?,
                )
                .map_err(|e| StorageIOError::write_logs(&e))?;
        }

        self.db.flush_wal(true).map_err(|e| StorageIOError::write_logs(&e))?;

        // If there is error, the callback will be dropped.
        callback.log_io_completed(Ok(()));
        Ok(())
    }

    async fn truncate(&mut self, log_id: LogId<RocksNodeId>) -> Result<(), StorageError<RocksNodeId>> {
        tracing::debug!("truncate: [{:?}, +oo)", log_id);

        let from = id_to_bin(log_id.index);
        let to = id_to_bin(0xff_ff_ff_ff_ff_ff_ff_ff);
        self.db.delete_range_cf(self.cf_logs(), &from, &to).map_err(|e| StorageIOError::write_logs(&e))?;

        self.db.flush_wal(true).map_err(|e| StorageIOError::write_logs(&e))?;
        Ok(())
    }

    async fn purge(&mut self, log_id: LogId<RocksNodeId>) -> Result<(), StorageError<RocksNodeId>> {
        tracing::debug!("delete_log: [0, {:?}]", log_id);

        // Write the last-purged log id before purging the logs.
        // The logs at and before last-purged log id will be ignored by openraft.
        // Therefore there is no need to do it in a transaction.
        self.put_meta::<meta::LastPurged>(&log_id)?;

        let from = id_to_bin(0);
        let to = id_to_bin(log_id.index + 1);
        self.db.delete_range_cf(self.cf_logs(), &from, &to).map_err(|e| StorageIOError::write_logs(&e))?;

        // Purging does not need to be persistent.
        Ok(())
    }
}

impl RaftStateMachine<TypeConfig> for RocksStateMachine {
    type SnapshotBuilder = Self;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogId<RocksNodeId>>, StoredMembership<TypeConfig>), StorageError<RocksNodeId>> {
        Ok((self.sm.last_applied_log, self.sm.last_membership.clone()))
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<RocksResponse>, StorageError<RocksNodeId>>
    where I: IntoIterator<Item = Entry<TypeConfig>> + Send {
        let entries_iter = entries.into_iter();
        let mut res = Vec::with_capacity(entries_iter.size_hint().0);

        let sm = &mut self.sm;

        for entry in entries_iter {
            tracing::debug!(%entry.log_id, "replicate to sm");

            sm.last_applied_log = Some(*entry.get_log_id());

            match entry.payload {
                EntryPayload::Blank => res.push(RocksResponse { value: None }),
                EntryPayload::Normal(ref req) => match req {
                    RocksRequest::Set { key, value } => {
                        sm.data.insert(key.clone(), value.clone());
                        res.push(RocksResponse {
                            value: Some(value.clone()),
                        })
                    }
                },
                EntryPayload::Membership(ref mem) => {
                    sm.last_membership = StoredMembership::new(Some(entry.log_id), mem.clone());
                    res.push(RocksResponse { value: None })
                }
            };
        }
        Ok(res)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn begin_receiving_snapshot(&mut self) -> Result<Box<SnapshotDataOf<TypeConfig>>, StorageError<RocksNodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<TypeConfig>,
        snapshot: Box<SnapshotDataOf<TypeConfig>>,
    ) -> Result<(), StorageError<RocksNodeId>> {
        tracing::info!(
            { snapshot_size = snapshot.get_ref().len() },
            "decoding snapshot for installation"
        );

        let new_snapshot = RocksSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        // Update the state machine.
        let updated_state_machine: StateMachine = serde_json::from_slice(&new_snapshot.data)
            .map_err(|e| StorageIOError::read_snapshot(Some(new_snapshot.meta.signature()), &e))?;

        self.sm = updated_state_machine;

        // Save snapshot

        let serialized_snapshot = serde_json::to_vec(&new_snapshot)
            .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), AnyError::new(&e)))?;

        self.db
            .put_cf(self.db.cf_handle("sm_meta").unwrap(), "snapshot", serialized_snapshot)
            .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), AnyError::new(&e)))?;

        self.db.flush_wal(true).map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), &e))?;
        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TypeConfig>>, StorageError<RocksNodeId>> {
        let x = self
            .db
            .get_cf(self.db.cf_handle("sm_meta").unwrap(), "snapshot")
            .map_err(|e| StorageIOError::write_snapshot(None, AnyError::new(&e)))?;

        let bytes = match x {
            Some(x) => x,
            None => return Ok(None),
        };

        let snapshot: RocksSnapshot =
            serde_json::from_slice(&bytes).map_err(|e| StorageIOError::write_snapshot(None, AnyError::new(&e)))?;

        let data = snapshot.data.clone();

        Ok(Some(Snapshot {
            meta: snapshot.meta,
            snapshot: Box::new(Cursor::new(data)),
        }))
    }
}

/// Create a pair of `RocksLogStore` and `RocksStateMachine` that are backed by a same rocks db
/// instance.
pub async fn new<P: AsRef<Path>>(db_path: P) -> (RocksLogStore, RocksStateMachine) {
    let mut db_opts = Options::default();
    db_opts.create_missing_column_families(true);
    db_opts.create_if_missing(true);

    let meta = ColumnFamilyDescriptor::new("meta", Options::default());
    let sm_meta = ColumnFamilyDescriptor::new("sm_meta", Options::default());
    let logs = ColumnFamilyDescriptor::new("logs", Options::default());

    let db = DB::open_cf_descriptors(&db_opts, db_path, vec![meta, sm_meta, logs]).unwrap();

    let db = Arc::new(db);
    (RocksLogStore { db: db.clone() }, RocksStateMachine::new(db).await)
}

fn read_logs_err(e: impl Error + 'static) -> StorageError<RocksNodeId> {
    StorageError::IO {
        source: StorageIOError::read_logs(&e),
    }
}
