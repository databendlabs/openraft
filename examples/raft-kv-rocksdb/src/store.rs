use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use openraft::storage::RaftStateMachine;
use openraft::AnyError;
use openraft::EntryPayload;
use openraft::ErrorVerb;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;
use openraft_rocksstore::log_store::RocksLogStore;
use rocksdb::ColumnFamily;
use rocksdb::ColumnFamilyDescriptor;
use rocksdb::Direction;
use rocksdb::IteratorMode;
use rocksdb::Options;
use rocksdb::DB;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::typ::*;
use crate::TypeConfig;

/**
 * Here you will set the types of request that will interact with the raft nodes.
 * For example the `Set` will be used to write data (key and value) to the raft database.
 * The `AddNode` will append a new node to the current existing shared list of nodes.
 * You will want to add any request that can write data in all nodes here.
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Request {
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
pub struct Response {
    pub value: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StoredSnapshot {
    pub meta: SnapshotMeta,

    /// The data of the state machine at the time of this snapshot.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct StateMachineStore {
    pub data: StateMachineData,

    /// snapshot index is not persisted in this example.
    ///
    /// It is only used as a suffix of snapshot id, and should be globally unique.
    /// In practice, using a timestamp in micro-second would be good enough.
    snapshot_idx: u64,

    /// State machine stores snapshot in db.
    db: Arc<DB>,
}

#[derive(Debug, Clone)]
pub struct StateMachineData {
    pub last_applied_log_id: Option<LogId>,

    pub last_membership: StoredMembership,

    /// State built from applying the raft logs
    pub kvs: Arc<RwLock<BTreeMap<String, String>>>,
}

impl RaftSnapshotBuilder<TypeConfig> for StateMachineStore {
    async fn build_snapshot(&mut self) -> Result<Snapshot, StorageError> {
        let last_applied_log = self.data.last_applied_log_id;
        let last_membership = self.data.last_membership.clone();

        let kv_json = {
            let kvs = self.data.kvs.read().await;
            serde_json::to_vec(&*kvs).map_err(|e| StorageError::read_state_machine(&e))?
        };

        let snapshot_id = if let Some(last) = last_applied_log {
            format!("{}-{}-{}", last.committed_leader_id(), last.index(), self.snapshot_idx)
        } else {
            format!("--{}", self.snapshot_idx)
        };

        let meta = SnapshotMeta {
            last_log_id: last_applied_log,
            last_membership,
            snapshot_id,
        };

        let snapshot = Snapshot {
            meta,
            snapshot: Cursor::new(kv_json),
        };

        self.save_snapshot(&snapshot).await?;

        Ok(snapshot)
    }
}

impl StateMachineStore {
    async fn new(db: Arc<DB>) -> Result<StateMachineStore, StorageError> {
        let mut sm = Self {
            data: StateMachineData {
                last_applied_log_id: None,
                last_membership: Default::default(),
                kvs: Arc::new(Default::default()),
            },
            snapshot_idx: 0,
            db,
        };

        let snapshot = sm.get_current_snapshot_()?;
        if let Some(snap) = snapshot {
            sm.update_state_machine_(snap).await?;
        }

        Ok(sm)
    }

    async fn update_state_machine_(&mut self, snapshot: StoredSnapshot) -> Result<(), StorageError> {
        let kvs: BTreeMap<String, String> = serde_json::from_slice(&snapshot.data)
            .map_err(|e| StorageError::read_snapshot(Some(snapshot.meta.signature()), &e))?;

        self.data.last_applied_log_id = snapshot.meta.last_log_id;
        self.data.last_membership = snapshot.meta.last_membership.clone();
        let mut x = self.data.kvs.write().await;
        *x = kvs;

        Ok(())
    }

    /// List all snapshots in the db, returns the last
    fn get_current_snapshot_(&self) -> StorageResult<Option<StoredSnapshot>> {
        let mut last_snapshot_key = None;

        let it = self.db.iterator_cf(self.store(), IteratorMode::From(b"snapshot-", Direction::Forward));

        for kv in it {
            let (key, _value) = kv.map_err(|e| StorageError::read(&e))?;
            if key.starts_with(b"snapshot-") {
                last_snapshot_key = Some(key.to_vec());
            } else {
                break;
            }
        }
        let Some(key) = last_snapshot_key else {
            return Ok(None);
        };

        let data = self.db.get_cf(self.store(), &key).map_err(|e| StorageError::read(&e))?.unwrap();

        let snap: StoredSnapshot = serde_json::from_slice(&data).map_err(|e| StorageError::read_snapshot(None, &e))?;

        Ok(Some(snap))
    }

    /// Save snapshot by last-log-id and when reading, get the last one as the current.
    ///
    /// So that writing an old one won't overwrite the newer one.
    /// In a real world application, the old ones should be cleaned.
    fn set_current_snapshot_(&self, snap: StoredSnapshot) -> StorageResult<()> {
        let last_log_id = snap.meta.last_log_id.as_ref();

        let id_str = Self::order_preserved_log_id_string(last_log_id);

        let key = format!("snapshot-{id_str}");
        self.db
            .put_cf(
                self.store(),
                key.as_bytes(),
                serde_json::to_vec(&snap).unwrap().as_slice(),
            )
            .map_err(|e| StorageError::write_snapshot(Some(snap.meta.signature()), &e))?;

        self.flush(ErrorSubject::Snapshot(Some(snap.meta.signature())), ErrorVerb::Write)?;
        Ok(())
    }

    fn flush(&self, subject: ErrorSubject, verb: ErrorVerb) -> Result<(), StorageError> {
        self.db.flush_wal(true).map_err(|e| StorageError::new(subject, verb, AnyError::new(&e)))?;
        Ok(())
    }

    fn store(&self) -> &ColumnFamily {
        self.db.cf_handle("store").unwrap()
    }

    fn order_preserved_log_id_string(log_id: Option<&LogId>) -> String {
        let term = log_id.map(|l| l.committed_leader_id().term).unwrap_or_default();
        let node_id = log_id.map(|l| l.committed_leader_id().node_id).unwrap_or_default();
        let index = log_id.map(|l| l.index).unwrap_or_default();

        format!("{:020}-{:020}-{:020}", term, node_id, index)
    }
}

impl RaftStateMachine<TypeConfig> for StateMachineStore {
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogId>, StoredMembership), StorageError> {
        Ok((self.data.last_applied_log_id, self.data.last_membership.clone()))
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<Response>, StorageError>
    where
        I: IntoIterator<Item = Entry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let entries = entries.into_iter();
        let mut replies = Vec::with_capacity(entries.size_hint().0);

        for ent in entries {
            self.data.last_applied_log_id = Some(ent.log_id);

            let mut resp_value = None;

            match ent.payload {
                EntryPayload::Blank => {}
                EntryPayload::Normal(req) => match req {
                    Request::Set { key, value } => {
                        resp_value = Some(value.clone());

                        let mut st = self.data.kvs.write().await;
                        st.insert(key, value);
                    }
                },
                EntryPayload::Membership(mem) => {
                    self.data.last_membership = StoredMembership::new(Some(ent.log_id), mem);
                }
            }

            replies.push(Response { value: resp_value });
        }
        Ok(replies)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.snapshot_idx += 1;
        self.clone()
    }

    async fn begin_receiving_snapshot(&mut self) -> Result<Cursor<Vec<u8>>, StorageError> {
        Ok(Cursor::new(Vec::new()))
    }

    async fn install_snapshot(&mut self, meta: &SnapshotMeta, snapshot: SnapshotData) -> Result<(), StorageError> {
        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        self.update_state_machine_(new_snapshot).await?;

        Ok(())
    }

    async fn save_snapshot(&mut self, snapshot: &Snapshot) -> Result<(), StorageError> {
        let new_snapshot = StoredSnapshot {
            meta: snapshot.meta.clone(),
            data: snapshot.snapshot.clone().into_inner(),
        };

        self.set_current_snapshot_(new_snapshot)?;

        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot>, StorageError> {
        let x = self.get_current_snapshot_()?;
        Ok(x.map(|s| Snapshot {
            meta: s.meta.clone(),
            snapshot: Cursor::new(s.data.clone()),
        }))
    }
}

type StorageResult<T> = Result<T, StorageError>;

pub(crate) async fn new_storage<P: AsRef<Path>>(db_path: P) -> (RocksLogStore<TypeConfig>, StateMachineStore) {
    let mut db_opts = Options::default();
    db_opts.create_missing_column_families(true);
    db_opts.create_if_missing(true);

    let store = ColumnFamilyDescriptor::new("store", Options::default());
    let meta = ColumnFamilyDescriptor::new("meta", Options::default());
    let logs = ColumnFamilyDescriptor::new("logs", Options::default());

    let db = DB::open_cf_descriptors(&db_opts, db_path, vec![store, meta, logs]).unwrap();
    let db = Arc::new(db);

    let log_store = RocksLogStore::new(db.clone());
    let sm_store = StateMachineStore::new(db).await.unwrap();

    (log_store, sm_store)
}
