use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Debug;
use std::io;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use futures::Stream;
use futures::TryStreamExt;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use openraft::EntryPayload;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;
use openraft_rocksstore::log_store::RocksLogStore;
use rocksdb::ColumnFamily;
use rocksdb::ColumnFamilyDescriptor;
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

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Request::Set { key, value, .. } => write!(f, "Set {{ key: {}, value: {} }}", key, value),
        }
    }
}

/**
 * Here you define the response type for client read/write requests.
 *
 * This Response type is used as the `AppDataResponse` in the `TypeConfig`.
 * It represents the result returned to clients after applying operations
 * to the state machine.
 *
 * In this example, it returns an optional value for a given key.
 *
 * ## Using Multiple Response Types
 *
 * For applications with diverse operations, you can use an enum:
 *
 * ```ignore
 * #[derive(Serialize, Deserialize, Debug, Clone)]
 * pub enum Response {
 *     Get { value: Option<String> },
 *     Set { prev_value: Option<String> },
 *     Delete { existed: bool },
 *     List { keys: Vec<String> },
 * }
 * ```
 *
 * Each variant corresponds to a different operation in your `Request` enum,
 * providing strongly-typed responses for different client operations.
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
    async fn build_snapshot(&mut self) -> Result<Snapshot, io::Error> {
        let last_applied_log = self.data.last_applied_log_id;
        let last_membership = self.data.last_membership.clone();

        let kv_json = {
            let kvs = self.data.kvs.read().await;
            serde_json::to_vec(&*kvs).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
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

        let snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: kv_json.clone(),
        };

        self.set_current_snapshot_(snapshot)?;

        Ok(Snapshot {
            meta,
            snapshot: Cursor::new(kv_json),
        })
    }
}

impl StateMachineStore {
    async fn new(db: Arc<DB>) -> Result<StateMachineStore, io::Error> {
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

    async fn update_state_machine_(&mut self, snapshot: StoredSnapshot) -> Result<(), io::Error> {
        let kvs: BTreeMap<String, String> =
            serde_json::from_slice(&snapshot.data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        self.data.last_applied_log_id = snapshot.meta.last_log_id;
        self.data.last_membership = snapshot.meta.last_membership.clone();
        let mut x = self.data.kvs.write().await;
        *x = kvs;

        Ok(())
    }

    fn get_current_snapshot_(&self) -> Result<Option<StoredSnapshot>, io::Error> {
        Ok(self
            .db
            .get_cf(self.store(), b"snapshot")
            .map_err(io::Error::other)?
            .and_then(|v| serde_json::from_slice(&v).ok()))
    }

    fn set_current_snapshot_(&self, snap: StoredSnapshot) -> Result<(), io::Error> {
        self.db
            .put_cf(self.store(), b"snapshot", serde_json::to_vec(&snap).unwrap().as_slice())
            .map_err(io::Error::other)?;
        self.db.flush_wal(true).map_err(io::Error::other)?;
        Ok(())
    }

    fn store(&self) -> &ColumnFamily {
        self.db.cf_handle("store").unwrap()
    }
}

impl RaftStateMachine<TypeConfig> for StateMachineStore {
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogId>, StoredMembership), io::Error> {
        Ok((self.data.last_applied_log_id, self.data.last_membership.clone()))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend {
        while let Some((entry, responder)) = entries.try_next().await? {
            self.data.last_applied_log_id = Some(entry.log_id);

            let response = match entry.payload {
                EntryPayload::Blank => Response { value: None },
                EntryPayload::Normal(req) => match req {
                    Request::Set { key, value } => {
                        let mut st = self.data.kvs.write().await;
                        st.insert(key, value.clone());
                        Response { value: Some(value) }
                    }
                },
                EntryPayload::Membership(mem) => {
                    self.data.last_membership = StoredMembership::new(Some(entry.log_id), mem);
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
        self.snapshot_idx += 1;
        self.clone()
    }

    async fn begin_receiving_snapshot(&mut self) -> Result<Cursor<Vec<u8>>, io::Error> {
        Ok(Cursor::new(Vec::new()))
    }

    async fn install_snapshot(&mut self, meta: &SnapshotMeta, snapshot: SnapshotData) -> Result<(), io::Error> {
        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        self.update_state_machine_(new_snapshot.clone()).await?;

        self.set_current_snapshot_(new_snapshot)?;

        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot>, io::Error> {
        let x = self.get_current_snapshot_()?;
        Ok(x.map(|s| Snapshot {
            meta: s.meta.clone(),
            snapshot: Cursor::new(s.data.clone()),
        }))
    }
}

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
