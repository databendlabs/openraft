#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]

#[cfg(test)] mod test;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;

use async_std::sync::RwLock;
use byteorder::BigEndian;
use byteorder::ByteOrder;
use byteorder::ReadBytesExt;
use openraft::async_trait::async_trait;
use openraft::storage::LogState;
use openraft::storage::Snapshot;
use openraft::AnyError;
use openraft::BasicNode;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::RaftLogId;
use openraft::RaftLogReader;
use openraft::RaftSnapshotBuilder;
use openraft::RaftStorage;
use openraft::RaftTypeConfig;
use openraft::SnapshotMeta;
use openraft::StorageError;
use openraft::StorageIOError;
use openraft::StoredMembership;
use openraft::Vote;
use serde::Deserialize;
use serde::Serialize;
use sled::Transactional;

pub type ExampleNodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig: D = ExampleRequest, R = ExampleResponse, NodeId = ExampleNodeId, Node = BasicNode,
    Entry = Entry<TypeConfig>, SnapshotData = Cursor<Vec<u8>>
);

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
 * TODO: Should we explain how to create multiple `AppDataResponse`?
 *
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExampleResponse {
    pub value: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExampleSnapshot {
    pub meta: SnapshotMeta<ExampleNodeId, BasicNode>,

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

    pub last_membership: StoredMembership<ExampleNodeId, BasicNode>,

    /// Application data.
    pub data: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ExampleStateMachine {
    /// Application data.
    pub db: Arc<sled::Db>,
}

impl From<&ExampleStateMachine> for SerializableExampleStateMachine {
    fn from(state: &ExampleStateMachine) -> Self {
        let mut data_tree = BTreeMap::new();
        for entry_res in data(&state.db).iter() {
            let entry = entry_res.expect("read db failed");

            let key: &[u8] = &entry.0;
            let value: &[u8] = &entry.1;
            data_tree.insert(
                String::from_utf8(key.to_vec()).expect("invalid key"),
                String::from_utf8(value.to_vec()).expect("invalid data"),
            );
        }
        Self {
            last_applied_log: state.get_last_applied_log().expect("last_applied_log"),
            last_membership: state.get_last_membership().expect("last_membership"),
            data: data_tree,
        }
    }
}

fn read_sm_err<E: Error + 'static>(e: E) -> StorageIOError<ExampleNodeId> {
    StorageIOError::read_state_machine(&e)
}
fn write_sm_err<E: Error + 'static>(e: E) -> StorageIOError<ExampleNodeId> {
    StorageIOError::write_state_machine(&e)
}
fn read_snap_err<E: Error + 'static>(e: E) -> StorageIOError<ExampleNodeId> {
    StorageIOError::read(&e)
}
fn write_snap_err<E: Error + 'static>(e: E) -> StorageIOError<ExampleNodeId> {
    StorageIOError::write(&e)
}
fn read_vote_err<E: Error + 'static>(e: E) -> StorageIOError<ExampleNodeId> {
    StorageIOError::read_vote(&e)
}
fn write_vote_err<E: Error + 'static>(e: E) -> StorageIOError<ExampleNodeId> {
    StorageIOError::write_vote(&e)
}
fn read_logs_err<E: Error + 'static>(e: E) -> StorageIOError<ExampleNodeId> {
    StorageIOError::read_logs(&e)
}
fn write_logs_err<E: Error + 'static>(e: E) -> StorageIOError<ExampleNodeId> {
    StorageIOError::write_logs(&e)
}
fn read_err<E: Error + 'static>(e: E) -> StorageIOError<ExampleNodeId> {
    StorageIOError::read(&e)
}
fn write_err<E: Error + 'static>(e: E) -> StorageIOError<ExampleNodeId> {
    StorageIOError::write(&e)
}

fn conflictable_txn_err<E: Error + 'static>(e: E) -> sled::transaction::ConflictableTransactionError<AnyError> {
    sled::transaction::ConflictableTransactionError::Abort(AnyError::new(&e))
}

impl ExampleStateMachine {
    fn get_last_membership(&self) -> StorageResult<StoredMembership<ExampleNodeId, BasicNode>> {
        let state_machine = state_machine(&self.db);
        let ivec = state_machine.get(b"last_membership").map_err(read_sm_err)?;

        let m = if let Some(ivec) = ivec {
            serde_json::from_slice(&ivec).map_err(read_sm_err)?
        } else {
            StoredMembership::default()
        };

        Ok(m)
    }
    async fn set_last_membership(&self, membership: StoredMembership<ExampleNodeId, BasicNode>) -> StorageResult<()> {
        let value = serde_json::to_vec(&membership).map_err(write_sm_err)?;
        let state_machine = state_machine(&self.db);
        state_machine.insert(b"last_membership", value).map_err(write_err)?;

        state_machine.flush_async().await.map_err(write_err)?;
        Ok(())
    }
    fn set_last_membership_tx(
        &self,
        tx_state_machine: &sled::transaction::TransactionalTree,
        membership: StoredMembership<ExampleNodeId, BasicNode>,
    ) -> Result<(), sled::transaction::ConflictableTransactionError<AnyError>> {
        let value = serde_json::to_vec(&membership).map_err(conflictable_txn_err)?;
        tx_state_machine.insert(b"last_membership", value).map_err(conflictable_txn_err)?;
        Ok(())
    }
    fn get_last_applied_log(&self) -> StorageResult<Option<LogId<ExampleNodeId>>> {
        let state_machine = state_machine(&self.db);
        let ivec = state_machine.get(b"last_applied_log").map_err(read_logs_err)?;
        let last_applied = if let Some(ivec) = ivec {
            serde_json::from_slice(&ivec).map_err(read_sm_err)?
        } else {
            None
        };
        Ok(last_applied)
    }
    async fn set_last_applied_log(&self, log_id: LogId<ExampleNodeId>) -> StorageResult<()> {
        let value = serde_json::to_vec(&log_id).map_err(write_sm_err)?;
        let state_machine = state_machine(&self.db);
        state_machine.insert(b"last_applied_log", value).map_err(read_logs_err)?;

        state_machine.flush_async().await.map_err(read_logs_err)?;
        Ok(())
    }
    fn set_last_applied_log_tx(
        &self,
        tx_state_machine: &sled::transaction::TransactionalTree,
        log_id: LogId<ExampleNodeId>,
    ) -> Result<(), sled::transaction::ConflictableTransactionError<AnyError>> {
        let value = serde_json::to_vec(&log_id).map_err(conflictable_txn_err)?;
        tx_state_machine.insert(b"last_applied_log", value).map_err(conflictable_txn_err)?;
        Ok(())
    }
    async fn from_serializable(sm: SerializableExampleStateMachine, db: Arc<sled::Db>) -> StorageResult<Self> {
        let data_tree = data(&db);
        let mut batch = sled::Batch::default();
        for (key, value) in sm.data {
            batch.insert(key.as_bytes(), value.as_bytes())
        }
        data_tree.apply_batch(batch).map_err(write_sm_err)?;
        data_tree.flush_async().await.map_err(write_snap_err)?;

        let r = Self { db };
        if let Some(log_id) = sm.last_applied_log {
            r.set_last_applied_log(log_id).await?;
        }
        r.set_last_membership(sm.last_membership).await?;

        Ok(r)
    }

    fn new(db: Arc<sled::Db>) -> ExampleStateMachine {
        Self { db }
    }
    fn insert_tx(
        &self,
        tx_data_tree: &sled::transaction::TransactionalTree,
        key: String,
        value: String,
    ) -> Result<(), sled::transaction::ConflictableTransactionError<AnyError>> {
        tx_data_tree.insert(key.as_bytes(), value.as_bytes()).map_err(conflictable_txn_err)?;
        Ok(())
    }
    pub fn get(&self, key: &str) -> StorageResult<Option<String>> {
        let key = key.as_bytes();
        let data_tree = data(&self.db);
        data_tree
            .get(key)
            .map(|value| value.map(|value| String::from_utf8(value.to_vec()).expect("invalid data")))
            .map_err(|e| StorageIOError::read(&e).into())
    }
    pub fn get_all(&self) -> StorageResult<Vec<String>> {
        let data_tree = data(&self.db);

        let data = data_tree
            .iter()
            .filter_map(|entry_res| {
                if let Ok(el) = entry_res {
                    Some(String::from_utf8(el.1.to_vec()).expect("invalid data"))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        Ok(data)
    }
}

#[derive(Debug)]
pub struct SledStore {
    db: Arc<sled::Db>,

    /// The Raft state machine.
    pub state_machine: RwLock<ExampleStateMachine>,
}

type StorageResult<T> = Result<T, StorageError<ExampleNodeId>>;

/// converts an id to a byte vector for storing in the database.
/// Note that we're using big endian encoding to ensure correct sorting of keys
/// with notes form: <https://github.com/spacejam/sled#a-note-on-lexicographic-ordering-and-endianness>
fn id_to_bin(id: u64) -> [u8; 8] {
    let mut buf: [u8; 8] = [0; 8];
    BigEndian::write_u64(&mut buf, id);
    buf
}

fn bin_to_id(buf: &[u8]) -> u64 {
    (&buf[0..8]).read_u64::<BigEndian>().unwrap()
}

impl SledStore {
    fn get_last_purged_(&self) -> StorageResult<Option<LogId<u64>>> {
        let store_tree = store(&self.db);
        let val = store_tree.get(b"last_purged_log_id").map_err(read_err)?;

        if let Some(v) = val {
            let val = serde_json::from_slice(&v).map_err(read_err)?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    async fn set_last_purged_(&self, log_id: LogId<u64>) -> StorageResult<()> {
        let store_tree = store(&self.db);
        let val = serde_json::to_vec(&log_id).unwrap();
        store_tree.insert(b"last_purged_log_id", val.as_slice()).map_err(write_snap_err)?;

        store_tree.flush_async().await.map_err(write_snap_err)?;
        Ok(())
    }

    fn get_snapshot_index_(&self) -> StorageResult<u64> {
        let store_tree = store(&self.db);
        let ivec = store_tree.get(b"snapshot_index").map_err(read_snap_err)?;

        if let Some(v) = ivec {
            let val = serde_json::from_slice(&v).map_err(read_err)?;
            Ok(val)
        } else {
            Ok(0)
        }
    }

    async fn set_snapshot_index_(&self, snapshot_index: u64) -> StorageResult<()> {
        let store_tree = store(&self.db);
        let val = serde_json::to_vec(&snapshot_index).unwrap();
        store_tree.insert(b"snapshot_index", val.as_slice()).map_err(write_snap_err)?;

        store_tree.flush_async().await.map_err(write_snap_err)?;
        Ok(())
    }

    async fn set_vote_(&self, vote: &Vote<ExampleNodeId>) -> Result<(), StorageError<ExampleNodeId>> {
        let store_tree = store(&self.db);
        let val = serde_json::to_vec(vote).unwrap();
        store_tree.insert(b"vote", val).map_err(write_vote_err)?;

        store_tree.flush_async().await.map_err(write_vote_err)?;

        Ok(())
    }

    fn get_vote_(&self) -> StorageResult<Option<Vote<ExampleNodeId>>> {
        let store_tree = store(&self.db);
        let val = store_tree.get(b"vote").map_err(read_vote_err)?;
        let ivec = if let Some(t) = val {
            t
        } else {
            return Ok(None);
        };

        let v = serde_json::from_slice(&ivec).map_err(read_vote_err)?;
        Ok(Some(v))
    }

    fn get_current_snapshot_(&self) -> StorageResult<Option<ExampleSnapshot>> {
        let store_tree = store(&self.db);
        let ivec = store_tree.get(b"snapshot").map_err(read_snap_err)?;

        if let Some(ivec) = ivec {
            let snap = serde_json::from_slice(&ivec).map_err(read_snap_err)?;
            Ok(Some(snap))
        } else {
            Ok(None)
        }
    }

    async fn set_current_snapshot_(&self, snap: ExampleSnapshot) -> StorageResult<()> {
        let store_tree = store(&self.db);
        let val = serde_json::to_vec(&snap).unwrap();
        let meta = snap.meta.clone();
        store_tree.insert(b"snapshot", val.as_slice()).map_err(|e| StorageError::IO {
            source: StorageIOError::write_snapshot(Some(snap.meta.signature()), &e),
        })?;

        store_tree
            .flush_async()
            .await
            .map_err(|e| StorageIOError::write_snapshot(Some(meta.signature()), &e).into())
            .map(|_| ())
    }
}

#[async_trait]
impl RaftLogReader<TypeConfig> for Arc<SledStore> {
    async fn get_log_state(&mut self) -> StorageResult<LogState<TypeConfig>> {
        let last_purged = self.get_last_purged_()?;

        let logs_tree = logs(&self.db);
        let last_ivec_kv = logs_tree.last().map_err(read_logs_err)?;
        let (_, ent_ivec) = if let Some(last) = last_ivec_kv {
            last
        } else {
            return Ok(LogState {
                last_purged_log_id: last_purged,
                last_log_id: last_purged,
            });
        };

        let last_ent = serde_json::from_slice::<Entry<TypeConfig>>(&ent_ivec).map_err(read_logs_err)?;
        let last_log_id = Some(*last_ent.get_log_id());

        let last_log_id = std::cmp::max(last_log_id, last_purged);
        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id,
        })
    }

    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &mut self,
        range: RB,
    ) -> StorageResult<Vec<Entry<TypeConfig>>> {
        let start_bound = range.start_bound();
        let start = match start_bound {
            std::ops::Bound::Included(x) => id_to_bin(*x),
            std::ops::Bound::Excluded(x) => id_to_bin(*x + 1),
            std::ops::Bound::Unbounded => id_to_bin(0),
        };
        let logs_tree = logs(&self.db);
        let logs = logs_tree
            .range::<&[u8], _>(start.as_slice()..)
            .map(|el_res| {
                let el = el_res.expect("Failed read log entry");
                let id = el.0;
                let val = el.1;
                let entry: StorageResult<Entry<_>> = serde_json::from_slice(&val).map_err(|e| StorageError::IO {
                    source: StorageIOError::read_logs(&e),
                });
                let id = bin_to_id(&id);

                assert_eq!(Ok(id), entry.as_ref().map(|e| e.log_id.index));
                (id, entry)
            })
            .take_while(|(id, _)| range.contains(id))
            .map(|x| x.1)
            .collect();
        logs
    }
}

#[async_trait]
impl RaftSnapshotBuilder<TypeConfig> for Arc<SledStore> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<ExampleNodeId>> {
        let data;
        let last_applied_log;
        let last_membership;

        {
            // Serialize the data of the state machine.
            let state_machine = SerializableExampleStateMachine::from(&*self.state_machine.read().await);
            data = serde_json::to_vec(&state_machine).map_err(|e| StorageIOError::read_state_machine(&e))?;

            last_applied_log = state_machine.last_applied_log;
            last_membership = state_machine.last_membership;
        }

        // TODO: we probably want this to be atomic.
        let snapshot_idx: u64 = self.get_snapshot_index_()? + 1;
        self.set_snapshot_index_(snapshot_idx).await?;

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

        let snapshot = ExampleSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        self.set_current_snapshot_(snapshot).await?;

        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

#[async_trait]
impl RaftStorage<TypeConfig> for Arc<SledStore> {
    type LogReader = Self;
    type SnapshotBuilder = Self;

    #[tracing::instrument(level = "trace", skip(self))]
    async fn save_vote(&mut self, vote: &Vote<ExampleNodeId>) -> Result<(), StorageError<ExampleNodeId>> {
        self.set_vote_(vote).await
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<ExampleNodeId>>, StorageError<ExampleNodeId>> {
        self.get_vote_()
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn append_to_log<I>(&mut self, entries: I) -> StorageResult<()>
    where I: IntoIterator<Item = Entry<TypeConfig>> + Send {
        let logs_tree = logs(&self.db);
        let mut batch = sled::Batch::default();
        for entry in entries {
            let id = id_to_bin(entry.log_id.index);
            assert_eq!(bin_to_id(&id), entry.log_id.index);
            let value = serde_json::to_vec(&entry).map_err(write_logs_err)?;
            batch.insert(id.as_slice(), value);
        }
        logs_tree.apply_batch(batch).map_err(write_logs_err)?;

        logs_tree.flush_async().await.map_err(write_logs_err)?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn delete_conflict_logs_since(&mut self, log_id: LogId<ExampleNodeId>) -> StorageResult<()> {
        tracing::debug!("delete_log: [{:?}, +oo)", log_id);

        let from = id_to_bin(log_id.index);
        let to = id_to_bin(0xff_ff_ff_ff_ff_ff_ff_ff);
        let logs_tree = logs(&self.db);
        let entries = logs_tree.range::<&[u8], _>(from.as_slice()..to.as_slice());
        let mut batch_del = sled::Batch::default();
        for entry_res in entries {
            let entry = entry_res.expect("Read db entry failed");
            batch_del.remove(entry.0);
        }
        logs_tree.apply_batch(batch_del).map_err(write_logs_err)?;
        logs_tree.flush_async().await.map_err(write_logs_err)?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn purge_logs_upto(&mut self, log_id: LogId<ExampleNodeId>) -> Result<(), StorageError<ExampleNodeId>> {
        tracing::debug!("delete_log: [0, {:?}]", log_id);

        self.set_last_purged_(log_id).await?;
        let from = id_to_bin(0);
        let to = id_to_bin(log_id.index);
        let logs_tree = logs(&self.db);
        let entries = logs_tree.range::<&[u8], _>(from.as_slice()..=to.as_slice());
        let mut batch_del = sled::Batch::default();
        for entry_res in entries {
            let entry = entry_res.expect("Read db entry failed");
            batch_del.remove(entry.0);
        }
        logs_tree.apply_batch(batch_del).map_err(write_logs_err)?;

        logs_tree.flush_async().await.map_err(write_logs_err)?;
        Ok(())
    }

    async fn last_applied_state(
        &mut self,
    ) -> Result<(Option<LogId<ExampleNodeId>>, StoredMembership<ExampleNodeId, BasicNode>), StorageError<ExampleNodeId>>
    {
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
    ) -> Result<Vec<ExampleResponse>, StorageError<ExampleNodeId>> {
        let sm = self.state_machine.write().await;
        let state_machine = state_machine(&self.db);
        let data_tree = data(&self.db);
        let trans_res = (&state_machine, &data_tree).transaction(|(tx_state_machine, tx_data_tree)| {
            let mut res = Vec::with_capacity(entries.len());

            for entry in entries {
                tracing::debug!(%entry.log_id, "replicate to sm");

                sm.set_last_applied_log_tx(tx_state_machine, entry.log_id)?;

                match entry.payload {
                    EntryPayload::Blank => res.push(ExampleResponse { value: None }),
                    EntryPayload::Normal(ref req) => match req {
                        ExampleRequest::Set { key, value } => {
                            sm.insert_tx(tx_data_tree, key.clone(), value.clone())?;
                            res.push(ExampleResponse {
                                value: Some(value.clone()),
                            })
                        }
                    },
                    EntryPayload::Membership(ref mem) => {
                        let membership = StoredMembership::new(Some(entry.log_id), mem.clone());
                        sm.set_last_membership_tx(tx_state_machine, membership)?;
                        res.push(ExampleResponse { value: None })
                    }
                };
            }
            Ok(res)
        });
        let result_vec = trans_res.map_err(|e| StorageIOError::write(&e))?;

        self.db.flush_async().await.map_err(|e| StorageIOError::write_logs(&e))?;
        Ok(result_vec)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<<TypeConfig as RaftTypeConfig>::SnapshotData>, StorageError<ExampleNodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<ExampleNodeId, BasicNode>,
        snapshot: Box<<TypeConfig as RaftTypeConfig>::SnapshotData>,
    ) -> Result<(), StorageError<ExampleNodeId>> {
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
                .map_err(|e| StorageIOError::read_snapshot(Some(new_snapshot.meta.signature()), &e))?;
            let mut state_machine = self.state_machine.write().await;
            *state_machine = ExampleStateMachine::from_serializable(updated_state_machine, self.db.clone()).await?;
        }

        self.set_current_snapshot_(new_snapshot).await?;
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TypeConfig>>, StorageError<ExampleNodeId>> {
        match SledStore::get_current_snapshot_(self)? {
            Some(snapshot) => {
                let data = snapshot.data.clone();
                Ok(Some(Snapshot {
                    meta: snapshot.meta,
                    snapshot: Box::new(Cursor::new(data)),
                }))
            }
            None => Ok(None),
        }
    }
}
impl SledStore {
    pub async fn new(db: Arc<sled::Db>) -> Arc<SledStore> {
        let _store = store(&db);
        let _state_machine = state_machine(&db);
        let _data = data(&db);
        let _logs = logs(&db);

        let state_machine = RwLock::new(ExampleStateMachine::new(db.clone()));
        Arc::new(SledStore { db, state_machine })
    }
}

fn store(db: &sled::Db) -> sled::Tree {
    db.open_tree("store").expect("store open failed")
}
fn logs(db: &sled::Db) -> sled::Tree {
    db.open_tree("logs").expect("logs open failed")
}
fn data(db: &sled::Db) -> sled::Tree {
    db.open_tree("data").expect("data open failed")
}
fn state_machine(db: &sled::Db) -> sled::Tree {
    db.open_tree("state_machine").expect("state_machine open failed")
}
