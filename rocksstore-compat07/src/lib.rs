#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]

//! This is an example implementation of the [`RaftStorage`] trait for an application that
//! needs to upgrade from openraft v0.7 to v0.8.
//!
//! Openraft v0.8 introduced several changes to the data types related to persistent data. This
//! example demonstrates how to upgrade the implementation of the storage without requiring any
//! modifications to the on-disk data, using the [`openraft::compat`] compatibility layer.
//!
//! This is a modified version of rocksstore that tries to
//! deserialize data into a compatible type, such as [`compat07::LogId`], when reading data from
//! rocksdb, and then upgrade it to the latest format. You can find usages of `compat07::*` that are
//! used in this implementation to provide compatibility with older data.
//!
//! [`RaftStorage`]: RaftStorage
//! [`openraft::compat`]: openraft::compat
//! [`compat07::LogId`]: compat07::LogId

#[cfg(test)] mod compatibility_test;
#[cfg(test)] mod test;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::path::Path;
use std::sync::Arc;

use async_std::sync::RwLock;
use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use byteorder::WriteBytesExt;
use openraft::async_trait::async_trait;
use openraft::compat::compat07;
use openraft::compat::Upgrade;
use openraft::AnyError;
use openraft::EmptyNode;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::ErrorSubject;
use openraft::ErrorVerb;
use openraft::LogId;
use openraft::LogState;
use openraft::RaftLogReader;
use openraft::RaftSnapshotBuilder;
use openraft::RaftStorage;
use openraft::RaftTypeConfig;
use openraft::Snapshot;
use openraft::SnapshotMeta;
use openraft::StorageError;
use openraft::StorageIOError;
use openraft::StoredMembership;
use openraft::TokioRuntime;
use openraft::Vote;
use rocksdb::ColumnFamily;
use rocksdb::ColumnFamilyDescriptor;
use rocksdb::Direction;
use rocksdb::Options;
use rocksdb::DB;
use serde::Deserialize;
use serde::Serialize;

pub type RocksNodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration for `MemStore`.
    pub TypeConfig: D = RocksRequest, R = RocksResponse, NodeId = RocksNodeId, Node = EmptyNode,
    Entry = Entry<TypeConfig>, SnapshotData = Cursor<Vec<u8>>, AsyncRuntime = TokioRuntime
);

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum RocksRequest {
    Set { key: String, value: String },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RocksResponse {
    pub value: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RocksSnapshot {
    pub meta: SnapshotMeta<RocksNodeId, EmptyNode>,
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RocksSnapshotCompat {
    pub meta: compat07::SnapshotMeta,
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct SerializableRocksStateMachine {
    pub last_applied_log: Option<LogId<RocksNodeId>>,
    pub last_membership: StoredMembership<RocksNodeId, EmptyNode>,
    pub data: BTreeMap<String, String>,
}

impl From<&RocksStateMachine> for SerializableRocksStateMachine {
    fn from(state: &RocksStateMachine) -> Self {
        let mut data = BTreeMap::new();

        let it = state.db.iterator_cf(state.cf_sm_data(), rocksdb::IteratorMode::Start);

        for item in it {
            let (key, value) = item.expect("invalid kv record");

            let key: &[u8] = &key;
            let value: &[u8] = &value;
            data.insert(
                String::from_utf8(key.to_vec()).expect("invalid key"),
                String::from_utf8(value.to_vec()).expect("invalid data"),
            );
        }
        Self {
            last_applied_log: state.get_last_applied_log().expect("last_applied_log"),
            last_membership: state.get_last_membership().expect("last_membership"),
            data,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RocksStateMachine {
    /// Application data.
    pub db: Arc<DB>,
}

fn sm_r_err<E: Error + 'static>(e: E) -> StorageError<RocksNodeId> {
    StorageIOError::read_state_machine(&e).into()
}
fn sm_w_err<E: Error + 'static>(e: E) -> StorageError<RocksNodeId> {
    StorageIOError::write_state_machine(&e).into()
}

impl RocksStateMachine {
    fn cf_sm_meta(&self) -> &ColumnFamily {
        self.db.cf_handle("sm_meta").unwrap()
    }

    fn cf_sm_data(&self) -> &ColumnFamily {
        self.db.cf_handle("sm_data").unwrap()
    }

    fn get_last_membership(&self) -> StorageResult<StoredMembership<RocksNodeId, EmptyNode>> {
        let bs = self.db.get_cf(self.cf_sm_meta(), b"last_membership").map_err(sm_r_err)?;
        let bs = if let Some(x) = bs {
            x
        } else {
            return Ok(StoredMembership::default());
        };

        let em = serde_json::from_slice::<compat07::StoredMembership>(&bs).map_err(sm_r_err)?;
        Ok(em.upgrade())
    }

    fn set_last_membership(&self, membership: StoredMembership<RocksNodeId, EmptyNode>) -> StorageResult<()> {
        self.db
            .put_cf(
                self.cf_sm_meta(),
                b"last_membership",
                serde_json::to_vec(&membership).map_err(sm_w_err)?,
            )
            .map_err(sm_w_err)
    }

    fn get_last_applied_log(&self) -> StorageResult<Option<LogId<RocksNodeId>>> {
        let bs = self.db.get_cf(self.cf_sm_meta(), b"last_applied_log").map_err(sm_r_err)?;
        let bs = if let Some(x) = bs {
            x
        } else {
            return Ok(None);
        };

        let log_id = serde_json::from_slice::<compat07::LogId>(&bs).map_err(sm_r_err)?;
        Ok(Some(log_id.upgrade()))
    }

    fn set_last_applied_log(&self, log_id: LogId<RocksNodeId>) -> StorageResult<()> {
        self.db
            .put_cf(
                self.cf_sm_meta(),
                "last_applied_log".as_bytes(),
                serde_json::to_vec(&log_id).map_err(sm_w_err)?,
            )
            .map_err(sm_w_err)
    }

    fn from_serializable(sm: SerializableRocksStateMachine, db: Arc<DB>) -> StorageResult<Self> {
        let r = Self { db };

        let cf = r.cf_sm_data();
        for (key, value) in sm.data {
            r.db.put_cf(cf, key.as_bytes(), value.as_bytes()).map_err(sm_w_err)?;
        }

        if let Some(log_id) = sm.last_applied_log {
            r.set_last_applied_log(log_id)?;
        }

        r.set_last_membership(sm.last_membership)?;

        Ok(r)
    }

    fn new(db: Arc<DB>) -> RocksStateMachine {
        Self { db }
    }

    fn insert(&self, key: String, value: String) -> StorageResult<()> {
        self.db
            .put_cf(self.cf_sm_data(), key.as_bytes(), value.as_bytes())
            .map_err(|e| StorageIOError::write(&e).into())
    }

    pub fn get(&self, key: &str) -> StorageResult<Option<String>> {
        let key = key.as_bytes();
        self.db
            .get_cf(self.cf_sm_data(), key)
            .map(|value| value.map(|v| String::from_utf8(v).expect("invalid data")))
            .map_err(|e| StorageIOError::read(&e).into())
    }
}

#[derive(Debug)]
pub struct RocksStore {
    db: Arc<DB>,

    /// The Raft state machine.
    pub state_machine: RwLock<RocksStateMachine>,
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
    use crate::RocksSnapshot;

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
    pub(crate) struct SnapshotIndex {}
    pub(crate) struct HardState {}
    pub(crate) struct Vote {}
    pub(crate) struct Snapshot {}

    impl StoreMeta for LastPurged {
        const KEY: &'static str = "last_purged_log_id";
        type Value = LogId<u64>;

        fn subject(_v: Option<&Self::Value>) -> ErrorSubject<RocksNodeId> {
            ErrorSubject::Store
        }
    }
    impl StoreMeta for SnapshotIndex {
        const KEY: &'static str = "snapshot_index";
        type Value = u64;

        fn subject(_v: Option<&Self::Value>) -> ErrorSubject<RocksNodeId> {
            ErrorSubject::Store
        }
    }
    impl StoreMeta for HardState {
        const KEY: &'static str = "hard_state";
        type Value = ();

        fn subject(_v: Option<&Self::Value>) -> ErrorSubject<RocksNodeId> {
            ErrorSubject::Vote
        }
    }
    impl StoreMeta for Vote {
        // hard_state is renamed to vote, to hold compatibility, store them by the same key.
        const KEY: &'static str = "hard_state";
        type Value = openraft::Vote<RocksNodeId>;

        fn subject(_v: Option<&Self::Value>) -> ErrorSubject<RocksNodeId> {
            ErrorSubject::Vote
        }
    }
    impl StoreMeta for Snapshot {
        const KEY: &'static str = "snapshot";
        type Value = RocksSnapshot;

        fn subject(_v: Option<&Self::Value>) -> ErrorSubject<RocksNodeId> {
            ErrorSubject::None
        }
    }
}

impl RocksStore {
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

    fn get_meta_vec<M: meta::StoreMeta>(&self) -> Result<Option<Vec<u8>>, StorageError<RocksNodeId>> {
        let v = self
            .db
            .get_cf(self.cf_meta(), M::KEY)
            .map_err(|e| StorageIOError::new(M::subject(None), ErrorVerb::Read, AnyError::new(&e)))?;

        Ok(v)
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

#[async_trait]
impl RaftLogReader<TypeConfig> for Arc<RocksStore> {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
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

            let entry = serde_json::from_slice::<compat07::Entry<TypeConfig>>(&val).map_err(read_logs_err)?;
            let entry = entry.upgrade();

            assert_eq!(id, entry.log_id.index);

            res.push(entry);
        }
        Ok(res)
    }
}

#[async_trait]
impl RaftSnapshotBuilder<TypeConfig> for Arc<RocksStore> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<RocksNodeId>> {
        let data;
        let last_applied_log;
        let last_membership;

        {
            // Serialize the data of the state machine.
            let state_machine = SerializableRocksStateMachine::from(&*self.state_machine.read().await);
            data = serde_json::to_vec(&state_machine).map_err(|e| StorageIOError::read_state_machine(&e))?;

            last_applied_log = state_machine.last_applied_log;
            last_membership = state_machine.last_membership;
        }

        // TODO: we probably want this to be atomic.
        let snapshot_idx: u64 = self.get_meta::<meta::SnapshotIndex>()?.unwrap_or_default() + 1;
        self.put_meta::<meta::SnapshotIndex>(&snapshot_idx)?;

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

        self.put_meta::<meta::Snapshot>(&snapshot)?;

        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

#[async_trait]
impl RaftStorage<TypeConfig> for Arc<RocksStore> {
    type LogReader = Self;
    type SnapshotBuilder = Self;

    async fn get_log_state(&mut self) -> StorageResult<LogState<TypeConfig>> {
        let last = self.db.iterator_cf(self.cf_logs(), rocksdb::IteratorMode::End).next();

        let last_log_id = match last {
            None => None,
            Some(res) => {
                let (_log_index, entry_bytes) = res.map_err(read_logs_err)?;

                let ent = serde_json::from_slice::<compat07::Entry<TypeConfig>>(&entry_bytes).map_err(read_logs_err)?;
                let ent = ent.upgrade();
                Some(ent.log_id)
            }
        };

        let last_purged_log_id = self.get_meta_vec::<meta::LastPurged>()?;
        let last_purged_log_id = match last_purged_log_id {
            None => None,
            Some(bs) => {
                let log_id = serde_json::from_slice::<compat07::LogId>(&bs).map_err(read_logs_err)?;
                Some(log_id.upgrade())
            }
        };

        let last_log_id = match last_log_id {
            None => last_purged_log_id,
            Some(x) => Some(x),
        };

        Ok(LogState {
            last_purged_log_id,
            last_log_id,
        })
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn save_vote(&mut self, vote: &Vote<RocksNodeId>) -> Result<(), StorageError<RocksNodeId>> {
        self.put_meta::<meta::Vote>(vote)
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<RocksNodeId>>, StorageError<RocksNodeId>> {
        // TODO: ?
        // Read by old key
        let bs = self.get_meta_vec::<meta::HardState>()?;
        let bs = if let Some(bs) = bs {
            bs
        } else {
            return Ok(None);
        };

        let hs = serde_json::from_slice::<compat07::Vote>(&bs).map_err(|e| StorageIOError::read_vote(&e))?;

        let vote = hs.upgrade();
        Ok(Some(vote))
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn append_to_log<I>(&mut self, entries: I) -> StorageResult<()>
    where I: IntoIterator<Item = Entry<TypeConfig>> + Send {
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
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn delete_conflict_logs_since(&mut self, log_id: LogId<RocksNodeId>) -> StorageResult<()> {
        tracing::debug!("delete_log: [{:?}, +oo)", log_id);

        let from = id_to_bin(log_id.index);
        let to = id_to_bin(0xff_ff_ff_ff_ff_ff_ff_ff);
        self.db
            .delete_range_cf(self.cf_logs(), &from, &to)
            .map_err(|e| StorageIOError::write_logs(&e).into())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn purge_logs_upto(&mut self, log_id: LogId<RocksNodeId>) -> Result<(), StorageError<RocksNodeId>> {
        tracing::debug!("delete_log: [0, {:?}]", log_id);

        self.put_meta::<meta::LastPurged>(&log_id)?;

        let from = id_to_bin(0);
        let to = id_to_bin(log_id.index + 1);
        self.db
            .delete_range_cf(self.cf_logs(), &from, &to)
            .map_err(|e| StorageIOError::write_logs(&e).into())
    }

    async fn last_applied_state(
        &mut self,
    ) -> Result<(Option<LogId<RocksNodeId>>, StoredMembership<RocksNodeId, EmptyNode>), StorageError<RocksNodeId>> {
        let state_machine = self.state_machine.read().await;
        Ok((
            state_machine.get_last_applied_log()?,
            state_machine.get_last_membership()?,
        ))
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn apply_to_state_machine(
        &mut self,
        entries: &[Entry<TypeConfig>],
    ) -> Result<Vec<RocksResponse>, StorageError<RocksNodeId>> {
        let mut res = Vec::with_capacity(entries.len());

        let sm = self.state_machine.write().await;

        for entry in entries {
            tracing::debug!(%entry.log_id, "replicate to sm");

            sm.set_last_applied_log(entry.log_id)?;

            match entry.payload {
                EntryPayload::Blank => res.push(RocksResponse { value: None }),
                EntryPayload::Normal(ref req) => match req {
                    RocksRequest::Set { key, value } => {
                        sm.insert(key.clone(), value.clone())?;
                        res.push(RocksResponse {
                            value: Some(value.clone()),
                        })
                    }
                },
                EntryPayload::Membership(ref mem) => {
                    sm.set_last_membership(StoredMembership::new(Some(entry.log_id), mem.clone()))?;
                    res.push(RocksResponse { value: None })
                }
            };
        }
        self.db.flush_wal(true).map_err(|e| StorageIOError::write_logs(&e))?;
        Ok(res)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<<TypeConfig as RaftTypeConfig>::SnapshotData>, StorageError<RocksNodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<RocksNodeId, EmptyNode>,
        snapshot: Box<<TypeConfig as RaftTypeConfig>::SnapshotData>,
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
        {
            let updated_state_machine: SerializableRocksStateMachine = serde_json::from_slice(&new_snapshot.data)
                .map_err(|e| {
                    StorageIOError::new(
                        ErrorSubject::Snapshot(Some(new_snapshot.meta.signature())),
                        ErrorVerb::Read,
                        AnyError::new(&e),
                    )
                })?;
            let mut state_machine = self.state_machine.write().await;
            *state_machine = RocksStateMachine::from_serializable(updated_state_machine, self.db.clone())?;
        }

        self.put_meta::<meta::Snapshot>(&new_snapshot)?;

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TypeConfig>>, StorageError<RocksNodeId>> {
        let curr_snap = self.get_meta_vec::<meta::Snapshot>()?;
        let bs = if let Some(x) = curr_snap {
            x
        } else {
            return Ok(None);
        };

        println!("bs: {:?}", String::from_utf8(bs.clone()).unwrap());

        let curr_snap = serde_json::from_slice::<RocksSnapshotCompat>(&bs).map_err(|e| StorageIOError::read(&e))?;

        let d = curr_snap.data;
        let meta = if let Ok(meta) = curr_snap.meta.try_upgrade() {
            meta
        } else {
            // SnapshotMeta can not be upgrade.
            // It does not have `last_membership` field and can not be installed by a follower.
            return Ok(None);
        };

        Ok(Some(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(d)),
        }))
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}

impl RocksStore {
    pub async fn new<P: AsRef<Path>>(db_path: P) -> Arc<RocksStore> {
        let mut db_opts = Options::default();
        db_opts.create_missing_column_families(true);
        db_opts.create_if_missing(true);

        let meta = ColumnFamilyDescriptor::new("meta", Options::default());
        let sm_meta = ColumnFamilyDescriptor::new("sm_meta", Options::default());
        let sm_data = ColumnFamilyDescriptor::new("sm_data", Options::default());
        let logs = ColumnFamilyDescriptor::new("logs", Options::default());

        let db = DB::open_cf_descriptors(&db_opts, db_path, vec![meta, sm_meta, sm_data, logs]).unwrap();

        let db = Arc::new(db);
        let state_machine = RwLock::new(RocksStateMachine::new(db.clone()));
        Arc::new(RocksStore { db, state_machine })
    }
}

fn read_logs_err(e: impl Error + 'static) -> StorageError<RocksNodeId> {
    StorageError::IO {
        source: StorageIOError::read_logs(&e),
    }
}
