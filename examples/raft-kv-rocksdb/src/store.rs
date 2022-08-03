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
use openraft::storage::LogState;
use openraft::storage::Snapshot;
use openraft::AnyError;
use openraft::EffectiveMembership;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::ErrorSubject;
use openraft::ErrorVerb;
use openraft::LogId;
use openraft::RaftLogReader;
use openraft::RaftSnapshotBuilder;
use openraft::RaftStorage;
use openraft::SnapshotMeta;
use openraft::StateMachineChanges;
use openraft::StorageError;
use openraft::StorageIOError;
use openraft::Vote;
use rocksdb::ColumnFamily;
use rocksdb::ColumnFamilyDescriptor;
use rocksdb::Direction;
use rocksdb::Options;
use rocksdb::DB;
use serde::Deserialize;
use serde::Serialize;

use crate::ExampleNode;
use crate::ExampleNodeId;
use crate::ExampleTypeConfig;

/**
 * Here you will set the types of request that will interact with the raft nodes.
 * For example the `Set` will be used to write data (key and value) to the raft database.
 * The `AddNode` will append a new node to the current existing shared list of nodes.
 * You will want to add any request that can write data in all nodes here.
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ExampleRequest {
    Set { key: String, value: String },
}

/**
 * Here you will defined what type of answer you expect from reading the data of a node.
 * In this example it will return a optional value from a given key in
 * the `ExampleRequest.Set`.
 *
 * TODO: SHould we explain how to create multiple `AppDataResponse`?
 *
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExampleResponse {
    pub value: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExampleSnapshot {
    pub meta: SnapshotMeta<ExampleNodeId, ExampleNode>,

    /// The data of the state machine at the time of this snapshot.
    pub data: Vec<u8>,
}

/**
 * Here defines a state machine of the raft, this state represents a copy of the data
 * between each node. Note that we are using `serde` to serialize the `data`, which has
 * a implementation to be serialized. Note that for this test we set both the key and
 * value as String, but you could set any type of value that has the serialization impl.
 */
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct SerializableExampleStateMachine {
    pub last_applied_log: Option<LogId<ExampleNodeId>>,

    // TODO: it should not be Option.
    pub last_membership: EffectiveMembership<ExampleNodeId, ExampleNode>,

    /// Application data.
    pub data: BTreeMap<String, String>,
}

impl From<&ExampleStateMachine> for SerializableExampleStateMachine {
    fn from(state: &ExampleStateMachine) -> Self {
        let mut data = BTreeMap::new();
        for (key, value) in state.db.iterator_cf(
            state.db.cf_handle("data").expect("cf_handle"),
            rocksdb::IteratorMode::Start,
        ) {
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
pub struct ExampleStateMachine {
    // TODO: it should not be Option.
    // pub last_membership: EffectiveMembership<ExampleNodeId, NodeData>,
    /// Application data.
    pub db: Arc<rocksdb::DB>,
}

fn sm_r_err<E: Error + 'static>(e: E) -> StorageError<ExampleNodeId> {
    StorageIOError::new(ErrorSubject::StateMachine, ErrorVerb::Read, AnyError::new(&e)).into()
}
fn sm_w_err<E: Error + 'static>(e: E) -> StorageError<ExampleNodeId> {
    StorageIOError::new(ErrorSubject::StateMachine, ErrorVerb::Write, AnyError::new(&e)).into()
}

impl ExampleStateMachine {
    fn get_last_membership(&self) -> StorageResult<EffectiveMembership<ExampleNodeId, ExampleNode>> {
        self.db
            .get_cf(
                self.db.cf_handle("state_machine").expect("cf_handle"),
                "last_membership".as_bytes(),
            )
            .map_err(sm_r_err)
            .and_then(|value| {
                value
                    .map(|v| serde_json::from_slice(&v).map_err(sm_r_err))
                    .unwrap_or_else(|| Ok(EffectiveMembership::default()))
            })
    }
    fn set_last_membership(&self, membership: EffectiveMembership<ExampleNodeId, ExampleNode>) -> StorageResult<()> {
        self.db
            .put_cf(
                self.db.cf_handle("state_machine").expect("cf_handle"),
                "last_membership".as_bytes(),
                serde_json::to_vec(&membership).map_err(sm_w_err)?,
            )
            .map_err(sm_w_err)
    }
    fn get_last_applied_log(&self) -> StorageResult<Option<LogId<ExampleNodeId>>> {
        self.db
            .get_cf(
                self.db.cf_handle("state_machine").expect("cf_handle"),
                "last_applied_log".as_bytes(),
            )
            .map_err(sm_r_err)
            .and_then(|value| value.map(|v| serde_json::from_slice(&v).map_err(sm_r_err)).transpose())
    }
    fn set_last_applied_log(&self, log_id: LogId<ExampleNodeId>) -> StorageResult<()> {
        self.db
            .put_cf(
                self.db.cf_handle("state_machine").expect("cf_handle"),
                "last_applied_log".as_bytes(),
                serde_json::to_vec(&log_id).map_err(sm_w_err)?,
            )
            .map_err(sm_w_err)
    }
    fn from_serializable(sm: SerializableExampleStateMachine, db: Arc<rocksdb::DB>) -> StorageResult<Self> {
        for (key, value) in sm.data {
            db.put_cf(db.cf_handle("data").unwrap(), key.as_bytes(), value.as_bytes()).map_err(sm_w_err)?;
        }
        let r = Self { db };
        if let Some(log_id) = sm.last_applied_log {
            r.set_last_applied_log(log_id)?;
        }
        r.set_last_membership(sm.last_membership)?;

        Ok(r)
    }

    fn new(db: Arc<rocksdb::DB>) -> ExampleStateMachine {
        Self { db }
    }
    fn insert(&self, key: String, value: String) -> StorageResult<()> {
        self.db
            .put_cf(self.db.cf_handle("data").unwrap(), key.as_bytes(), value.as_bytes())
            .map_err(|e| StorageIOError::new(ErrorSubject::Store, ErrorVerb::Write, AnyError::new(&e)).into())
    }
    pub fn get(&self, key: &str) -> StorageResult<Option<String>> {
        let key = key.as_bytes();
        self.db
            .get_cf(self.db.cf_handle("data").unwrap(), key)
            .map(|value| {
                if let Some(value) = value {
                    Some(String::from_utf8(value.to_vec()).expect("invalid data"))
                } else {
                    None
                }
            })
            .map_err(|e| StorageIOError::new(ErrorSubject::Store, ErrorVerb::Read, AnyError::new(&e)).into())
    }
}

#[derive(Debug)]
pub struct ExampleStore {
    db: Arc<rocksdb::DB>,

    /// The Raft state machine.
    pub state_machine: RwLock<ExampleStateMachine>,
}
type StorageResult<T> = Result<T, StorageError<ExampleNodeId>>;

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

impl ExampleStore {
    fn store(&self) -> &ColumnFamily {
        self.db.cf_handle("store").unwrap()
    }
    fn logs(&self) -> &ColumnFamily {
        self.db.cf_handle("logs").unwrap()
    }
    fn get_last_purged_(&self) -> StorageResult<Option<LogId<u64>>> {
        Ok(self
            .db
            .get_cf(self.store(), b"last_purged_log_id")
            .map_err(|e| StorageIOError::new(ErrorSubject::Store, ErrorVerb::Read, AnyError::new(&e)))?
            .and_then(|v| serde_json::from_slice(&v).ok()))
    }

    fn set_last_purged_(&self, log_id: LogId<u64>) -> StorageResult<()> {
        self.db
            .put_cf(
                self.store(),
                b"last_purged_log_id",
                serde_json::to_vec(&log_id).unwrap().as_slice(),
            )
            .map_err(|e| StorageIOError::new(ErrorSubject::Store, ErrorVerb::Write, AnyError::new(&e)).into())
    }

    fn get_snapshot_index_(&self) -> StorageResult<u64> {
        Ok(self
            .db
            .get_cf(self.store(), b"snapshot_index")
            .map_err(|e| StorageIOError::new(ErrorSubject::Store, ErrorVerb::Read, AnyError::new(&e)))?
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or(0))
    }

    fn set_snapshot_indesx_(&self, snapshot_index: u64) -> StorageResult<()> {
        self.db
            .put_cf(
                self.store(),
                b"snapshot_index",
                serde_json::to_vec(&snapshot_index).unwrap().as_slice(),
            )
            .map_err(|e| StorageError::IO {
                source: StorageIOError::new(ErrorSubject::Store, ErrorVerb::Write, AnyError::new(&e)),
            })?;
        Ok(())
    }

    fn set_vote_(&self, vote: &Vote<ExampleNodeId>) -> StorageResult<()> {
        self.db
            .put_cf(self.store(), b"vote", serde_json::to_vec(vote).unwrap())
            .map_err(|e| StorageError::IO {
                source: StorageIOError::new(ErrorSubject::Vote, ErrorVerb::Write, AnyError::new(&e)),
            })
    }

    fn get_vote_(&self) -> StorageResult<Option<Vote<ExampleNodeId>>> {
        Ok(self
            .db
            .get_cf(self.store(), b"vote")
            .map_err(|e| StorageError::IO {
                source: StorageIOError::new(ErrorSubject::Vote, ErrorVerb::Write, AnyError::new(&e)),
            })?
            .and_then(|v| serde_json::from_slice(&v).ok()))
    }

    fn get_current_snapshot_(&self) -> StorageResult<Option<ExampleSnapshot>> {
        Ok(self
            .db
            .get_cf(self.store(), b"snapshot")
            .map_err(|e| StorageError::IO {
                source: StorageIOError::new(ErrorSubject::Store, ErrorVerb::Read, AnyError::new(&e)),
            })?
            .and_then(|v| serde_json::from_slice(&v).ok()))
    }

    fn set_current_snapshot_(&self, snap: ExampleSnapshot) -> StorageResult<()> {
        self.db
            .put_cf(self.store(), b"snapshot", serde_json::to_vec(&snap).unwrap().as_slice())
            .map_err(|e| StorageError::IO {
                source: StorageIOError::new(
                    ErrorSubject::Snapshot(snap.meta.signature()),
                    ErrorVerb::Write,
                    AnyError::new(&e),
                ),
            })?;
        Ok(())
    }
}

#[async_trait]
impl RaftLogReader<ExampleTypeConfig> for Arc<ExampleStore> {
    async fn get_log_state(&mut self) -> StorageResult<LogState<ExampleTypeConfig>> {
        let last = self
            .db
            .iterator_cf(self.logs(), rocksdb::IteratorMode::End)
            .next()
            .and_then(|(_, ent)| Some(serde_json::from_slice::<Entry<ExampleTypeConfig>>(&ent).ok()?.log_id));

        let last_purged_log_id = self.get_last_purged_()?;

        let last_log_id = match last {
            None => last_purged_log_id,
            Some(x) => Some(x),
        };
        Ok(LogState {
            last_purged_log_id,
            last_log_id,
        })
    }

    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &mut self,
        range: RB,
    ) -> StorageResult<Vec<Entry<ExampleTypeConfig>>> {
        let start = match range.start_bound() {
            std::ops::Bound::Included(x) => id_to_bin(*x),
            std::ops::Bound::Excluded(x) => id_to_bin(*x + 1),
            std::ops::Bound::Unbounded => id_to_bin(0),
        };
        self.db
            .iterator_cf(self.logs(), rocksdb::IteratorMode::From(&start, Direction::Forward))
            .map(|(id, val)| {
                let entry: StorageResult<Entry<_>> = serde_json::from_slice(&val).map_err(|e| StorageError::IO {
                    source: StorageIOError::new(ErrorSubject::Logs, ErrorVerb::Read, AnyError::new(&e)),
                });
                let id = bin_to_id(&id);

                assert_eq!(Ok(id), entry.as_ref().map(|e| e.log_id.index));
                (id, entry)
            })
            .take_while(|(id, _)| range.contains(&id))
            .map(|x| x.1)
            .collect()
    }
}

#[async_trait]
impl RaftSnapshotBuilder<ExampleTypeConfig, Cursor<Vec<u8>>> for Arc<ExampleStore> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(
        &mut self,
    ) -> Result<Snapshot<ExampleNodeId, ExampleNode, Cursor<Vec<u8>>>, StorageError<ExampleNodeId>> {
        let data;
        let last_applied_log;
        let last_membership;

        {
            // Serialize the data of the state machine.
            let state_machine = SerializableExampleStateMachine::from(&*self.state_machine.read().await);
            data = serde_json::to_vec(&state_machine)
                .map_err(|e| StorageIOError::new(ErrorSubject::StateMachine, ErrorVerb::Read, AnyError::new(&e)))?;

            last_applied_log = state_machine.last_applied_log;
            last_membership = state_machine.last_membership.clone();
        }

        let last_applied_log = match last_applied_log {
            None => {
                panic!("can not compact empty state machine");
            }
            Some(x) => x,
        };

        // TODO: we probably want thius to be atomic.
        let snapshot_idx: u64 = self.get_snapshot_index_()? + 1;
        self.set_snapshot_indesx_(snapshot_idx)?;

        let snapshot_id = format!(
            "{}-{}-{}",
            last_applied_log.leader_id, last_applied_log.index, snapshot_idx
        );

        let meta = SnapshotMeta {
            last_log_id: last_applied_log,
            last_membership,
            snapshot_id,
        };

        let snapshot = ExampleSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        self.set_current_snapshot_(snapshot)?;

        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

#[async_trait]
impl RaftStorage<ExampleTypeConfig> for Arc<ExampleStore> {
    type SnapshotData = Cursor<Vec<u8>>;
    type LogReader = Self;
    type SnapshotBuilder = Self;

    #[tracing::instrument(level = "trace", skip(self))]
    async fn save_vote(&mut self, vote: &Vote<ExampleNodeId>) -> Result<(), StorageError<ExampleNodeId>> {
        self.set_vote_(vote)
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<ExampleNodeId>>, StorageError<ExampleNodeId>> {
        self.get_vote_()
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn append_to_log(&mut self, entries: &[&Entry<ExampleTypeConfig>]) -> StorageResult<()> {
        for entry in entries {
            let id = id_to_bin(entry.log_id.index);
            assert_eq!(bin_to_id(&id), entry.log_id.index);
            self.db
                .put_cf(
                    self.logs(),
                    id,
                    serde_json::to_vec(entry)
                        .map_err(|e| StorageIOError::new(ErrorSubject::Logs, ErrorVerb::Write, AnyError::new(&e)))?,
                )
                .map_err(|e| StorageIOError::new(ErrorSubject::Logs, ErrorVerb::Write, AnyError::new(&e)))?;
        }
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn delete_conflict_logs_since(&mut self, log_id: LogId<ExampleNodeId>) -> StorageResult<()> {
        tracing::debug!("delete_log: [{:?}, +oo)", log_id);

        let from = id_to_bin(log_id.index);
        let to = id_to_bin(0xff_ff_ff_ff_ff_ff_ff_ff);
        self.db
            .delete_range_cf(self.logs(), &from, &to)
            .map_err(|e| StorageIOError::new(ErrorSubject::Logs, ErrorVerb::Write, AnyError::new(&e)).into())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn purge_logs_upto(&mut self, log_id: LogId<ExampleNodeId>) -> Result<(), StorageError<ExampleNodeId>> {
        tracing::debug!("delete_log: [0, {:?}]", log_id);

        self.set_last_purged_(log_id)?;
        let from = id_to_bin(0);
        let to = id_to_bin(log_id.index + 1);
        self.db
            .delete_range_cf(self.logs(), &from, &to)
            .map_err(|e| StorageIOError::new(ErrorSubject::Logs, ErrorVerb::Write, AnyError::new(&e)).into())
    }

    async fn last_applied_state(
        &mut self,
    ) -> Result<
        (
            Option<LogId<ExampleNodeId>>,
            EffectiveMembership<ExampleNodeId, ExampleNode>,
        ),
        StorageError<ExampleNodeId>,
    > {
        let state_machine = self.state_machine.read().await;
        Ok((
            state_machine.get_last_applied_log()?,
            state_machine.get_last_membership()?,
        ))
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn apply_to_state_machine(
        &mut self,
        entries: &[&Entry<ExampleTypeConfig>],
    ) -> Result<Vec<ExampleResponse>, StorageError<ExampleNodeId>> {
        let mut res = Vec::with_capacity(entries.len());

        let sm = self.state_machine.write().await;

        for entry in entries {
            tracing::debug!(%entry.log_id, "replicate to sm");

            sm.set_last_applied_log(entry.log_id)?;

            match entry.payload {
                EntryPayload::Blank => res.push(ExampleResponse { value: None }),
                EntryPayload::Normal(ref req) => match req {
                    ExampleRequest::Set { key, value } => {
                        sm.insert(key.clone(), value.clone())?;
                        res.push(ExampleResponse {
                            value: Some(value.clone()),
                        })
                    }
                },
                EntryPayload::Membership(ref mem) => {
                    sm.set_last_membership(EffectiveMembership::new(Some(entry.log_id), mem.clone()))?;
                    res.push(ExampleResponse { value: None })
                }
            };
        }
        self.db
            .flush_wal(true)
            .map_err(|e| StorageIOError::new(ErrorSubject::Logs, ErrorVerb::Write, AnyError::new(&e)))?;
        Ok(res)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&mut self) -> Result<Box<Self::SnapshotData>, StorageError<ExampleNodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<ExampleNodeId, ExampleNode>,
        snapshot: Box<Self::SnapshotData>,
    ) -> Result<StateMachineChanges<ExampleTypeConfig>, StorageError<ExampleNodeId>> {
        tracing::info!(
            { snapshot_size = snapshot.get_ref().len() },
            "decoding snapshot for installation"
        );

        let new_snapshot = ExampleSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        // Update the state machine.
        {
            let updated_state_machine: SerializableExampleStateMachine = serde_json::from_slice(&new_snapshot.data)
                .map_err(|e| {
                    StorageIOError::new(
                        ErrorSubject::Snapshot(new_snapshot.meta.signature()),
                        ErrorVerb::Read,
                        AnyError::new(&e),
                    )
                })?;
            let mut state_machine = self.state_machine.write().await;
            *state_machine = ExampleStateMachine::from_serializable(updated_state_machine, self.db.clone())?;
        }

        self.set_current_snapshot_(new_snapshot)?;
        Ok(StateMachineChanges {
            last_applied: meta.last_log_id,
            is_snapshot: true,
        })
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<ExampleNodeId, ExampleNode, Self::SnapshotData>>, StorageError<ExampleNodeId>> {
        match ExampleStore::get_current_snapshot_(self)? {
            Some(snapshot) => {
                let data = snapshot.data.clone();
                Ok(Some(Snapshot {
                    meta: snapshot.meta.clone(),
                    snapshot: Box::new(Cursor::new(data)),
                }))
            }
            None => Ok(None),
        }
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}
impl ExampleStore {
    pub(crate) async fn new<P: AsRef<Path>>(db_path: P) -> Arc<ExampleStore> {
        let mut db_opts = Options::default();
        db_opts.create_missing_column_families(true);
        db_opts.create_if_missing(true);

        let store = ColumnFamilyDescriptor::new("store", Options::default());
        let state_machine = ColumnFamilyDescriptor::new("state_machine", Options::default());
        let data = ColumnFamilyDescriptor::new("data", Options::default());
        let logs = ColumnFamilyDescriptor::new("logs", Options::default());

        let db = DB::open_cf_descriptors(&db_opts, db_path, vec![store, state_machine, data, logs]).unwrap();

        let db = Arc::new(db);
        let state_machine = RwLock::new(ExampleStateMachine::new(db.clone()));
        Arc::new(ExampleStore { db, state_machine })
    }
}
