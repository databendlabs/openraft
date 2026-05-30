use std::collections::BTreeMap;
use std::io;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use fjall::Database;
use fjall::Keyspace;
use fjall::KeyspaceCreateOptions;
use fjall::PersistMode;
use futures::Stream;
use futures::TryStreamExt;
use futures::lock::Mutex;
use openraft::EntryPayload;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use serde::Deserialize;
use serde::Serialize;

use crate::TypeConfig;
use crate::log_store::FjallLogStore;
use crate::typ::*;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StoredSnapshot {
    pub meta: SnapshotMeta,

    /// The data of the state machine at the time of this snapshot.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct StateMachineData {
    pub last_applied_log_id: Option<LogId>,

    pub last_membership: StoredMembership,

    /// State built from applying the raft logs
    pub kvs: Arc<Mutex<BTreeMap<String, String>>>,
}

#[derive(Clone)]
pub struct StateMachineStore {
    pub data: StateMachineData,
    /// snapshot index is not persisted in this example.
    ///
    /// It is only used as a suffix of snapshot id, and should be globally unique.
    /// In practice, using a timestamp in micro-second would be good enough.
    snapshot_idx: u64,

    /// State machine stores snapshot in db.
    db: Database,
}

impl RaftSnapshotBuilder<TypeConfig> for StateMachineStore {
    async fn build_snapshot(&mut self) -> Result<Snapshot, io::Error> {
        let last_applied_log = self.data.last_applied_log_id;
        let last_membership = self.data.last_membership.clone();

        let kv_json = {
            let kvs = self.data.kvs.lock().await;
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
    async fn new(db: Database) -> Result<Self, io::Error> {
        let mut sm = Self {
            data: StateMachineData {
                last_applied_log_id: None,
                last_membership: Default::default(),
                kvs: Arc::new(Mutex::new(BTreeMap::new())),
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

    fn get_current_snapshot_(&self) -> Result<Option<StoredSnapshot>, io::Error> {
        Ok(self
            .store()
            .get(b"snapshot")
            .map_err(io::Error::other)?
            .and_then(|v| serde_json::from_slice(v.as_ref()).ok()))
    }

    async fn update_state_machine_(&mut self, snapshot: StoredSnapshot) -> Result<(), io::Error> {
        let kvs: BTreeMap<String, String> =
            serde_json::from_slice(&snapshot.data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        self.data.last_applied_log_id = snapshot.meta.last_log_id;
        self.data.last_membership = snapshot.meta.last_membership.clone();
        let mut x = self.data.kvs.lock().await;
        *x = kvs;

        Ok(())
    }

    fn set_current_snapshot_(&self, snap: StoredSnapshot) -> Result<(), io::Error> {
        self.store()
            .insert(b"snapshot", serde_json::to_vec(&snap).unwrap().as_slice())
            .map_err(io::Error::other)?;
        self.db.persist(PersistMode::SyncAll).map_err(io::Error::other)?;
        Ok(())
    }

    fn store(&self) -> Keyspace {
        self.db.keyspace("store", KeyspaceCreateOptions::default).unwrap()
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
                EntryPayload::Blank => types_kv::Response::none(),
                EntryPayload::Normal(req) => match req {
                    types_kv::Request::Set { key, value } => {
                        let mut x = self.data.kvs.lock().await;
                        x.insert(key, value);
                        types_kv::Response::none()
                    }
                },
                EntryPayload::Membership(mem) => {
                    self.data.last_membership = StoredMembership::new(Some(entry.log_id), mem);
                    types_kv::Response::none()
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

pub(crate) async fn new_storage<P: AsRef<Path>>(db_path: P) -> (FjallLogStore<TypeConfig>, StateMachineStore) {
    let db = Database::builder(db_path).open().unwrap();

    let log_store = FjallLogStore::new(db.clone());
    let sm_store = StateMachineStore::new(db).await.unwrap();

    (log_store, sm_store)
}
