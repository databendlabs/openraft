use std::collections::BTreeMap;
use std::io;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use futures::Stream;
use futures::TryStreamExt;
use futures::lock::Mutex;
use log_rocks::RocksLogStore;
use openraft::EntryPayload;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use rocksdb::ColumnFamily;
use rocksdb::ColumnFamilyDescriptor;
use rocksdb::DB;
use rocksdb::Options;
use serde::Deserialize;
use serde::Serialize;

use crate::TypeConfig;
use crate::typ::*;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StoredSnapshot {
    pub meta: SnapshotMeta,

    /// The data of the state machine at the time of this snapshot.
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct StateMachineStore {
    pub data: StateMachineData,

    /// State machine stores snapshot in db.
    snapshots: SnapshotStore,
}

/// The stored snapshot, behind a lock that makes each read-compare-write atomic.
#[derive(Debug, Clone)]
struct SnapshotStore {
    db: Arc<DB>,

    /// Held while a snapshot is read and replaced. A builder runs in its own task, so without it
    /// a build could check the stored snapshot, lose the race to an install, and then overwrite
    /// the newer installed snapshot.
    lock: Arc<std::sync::Mutex<()>>,
}

#[derive(Debug, Clone)]
pub struct StateMachineData {
    pub last_applied_log_id: Option<LogId>,

    pub last_membership: StoredMembership,

    /// State built from applying the raft logs
    pub kvs: Arc<Mutex<BTreeMap<String, types_kv::VersionedValue>>>,
}

/// A frozen copy of the state machine, taken when the build was requested.
///
/// OpenRaft runs `build_snapshot` in a spawned task while later entries keep applying, so the
/// builder must not share the live map: its data could then be newer than `meta.last_log_id`.
pub struct SnapshotBuilder {
    meta: SnapshotMeta,
    kvs: BTreeMap<String, types_kv::VersionedValue>,
    snapshots: SnapshotStore,
}

impl RaftSnapshotBuilder<TypeConfig> for SnapshotBuilder {
    type SnapshotData = Cursor<Vec<u8>>;

    async fn build_snapshot(&mut self) -> Result<Snapshot, io::Error> {
        let kv_json = serde_json::to_vec(&self.kvs).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let snapshot = StoredSnapshot {
            meta: self.meta.clone(),
            data: kv_json,
        };

        if !self.snapshots.save_if_newer(&snapshot)? {
            // OpenRaft ignores a built snapshot that is not newer than its current one.
            tracing::info!(
                "kept the stored snapshot, which is at least as new as the built one: {:?}",
                snapshot.meta.last_log_id
            );
        }

        Ok(Snapshot {
            meta: snapshot.meta,
            snapshot: Cursor::new(snapshot.data),
        })
    }
}

impl SnapshotStore {
    fn get(&self) -> Result<Option<StoredSnapshot>, io::Error> {
        Ok(self
            .db
            .get_cf(self.cf(), b"snapshot")
            .map_err(io::Error::other)?
            .and_then(|v| serde_json::from_slice(&v).ok()))
    }

    /// Stores an installed snapshot. OpenRaft installs only a snapshot newer than this node's
    /// committed log, so it is newer than any snapshot built here.
    fn save(&self, snap: &StoredSnapshot) -> Result<(), io::Error> {
        let _guard = self.lock.lock().map_err(|_| io::Error::other("snapshot lock poisoned"))?;
        self.put(snap)
    }

    /// Stores a built snapshot unless the stored one is at least as new, and reports whether it
    /// was stored.
    fn save_if_newer(&self, snap: &StoredSnapshot) -> Result<bool, io::Error> {
        let _guard = self.lock.lock().map_err(|_| io::Error::other("snapshot lock poisoned"))?;
        if let Some(stored) = self.get()?
            && stored.meta.last_log_id >= snap.meta.last_log_id
        {
            return Ok(false);
        }
        self.put(snap)?;
        Ok(true)
    }

    fn put(&self, snap: &StoredSnapshot) -> Result<(), io::Error> {
        self.db
            .put_cf(self.cf(), b"snapshot", serde_json::to_vec(snap).unwrap().as_slice())
            .map_err(io::Error::other)?;
        self.db.flush_wal(true).map_err(io::Error::other)?;
        Ok(())
    }

    fn cf(&self) -> &ColumnFamily {
        self.db.cf_handle("store").unwrap()
    }
}

impl StateMachineStore {
    async fn new(db: Arc<DB>) -> Result<StateMachineStore, io::Error> {
        let mut sm = Self {
            data: StateMachineData {
                last_applied_log_id: None,
                last_membership: Default::default(),
                kvs: Arc::new(Mutex::new(BTreeMap::new())),
            },
            snapshots: SnapshotStore {
                db,
                lock: Arc::new(std::sync::Mutex::new(())),
            },
        };

        let snapshot = sm.snapshots.get()?;
        if let Some(snap) = snapshot {
            sm.update_state_machine_(snap).await?;
        }

        Ok(sm)
    }

    async fn update_state_machine_(&mut self, snapshot: StoredSnapshot) -> Result<(), io::Error> {
        let kvs: BTreeMap<String, types_kv::VersionedValue> =
            serde_json::from_slice(&snapshot.data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        self.data.last_applied_log_id = snapshot.meta.last_log_id;
        self.data.last_membership = snapshot.meta.last_membership.clone();
        let mut x = self.data.kvs.lock().await;
        *x = kvs;

        Ok(())
    }
}

impl RaftStateMachine<TypeConfig> for StateMachineStore {
    type SnapshotData = Cursor<Vec<u8>>;

    type SnapshotBuilder = SnapshotBuilder;

    async fn applied_state(&mut self) -> Result<(Option<LogId>, StoredMembership), io::Error> {
        Ok((self.data.last_applied_log_id.clone(), self.data.last_membership.clone()))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend {
        while let Some((entry, responder)) = entries.try_next().await? {
            let version = entry.log_id.index();
            self.data.last_applied_log_id = Some(entry.log_id.clone());

            let response = match entry.payload {
                EntryPayload::Blank => types_kv::Response::none(),
                EntryPayload::Normal(req) => match req {
                    types_kv::Request::Set { key, value } => {
                        let mut st = self.data.kvs.lock().await;
                        st.insert(key, types_kv::VersionedValue {
                            value: value.clone(),
                            version,
                        });
                        types_kv::Response::new(value, version)
                    }
                    types_kv::Request::CompareAndSet {
                        key,
                        expected_version,
                        value,
                    } => {
                        let mut st = self.data.kvs.lock().await;
                        let matches = st.get(&key).is_some_and(|current| current.version == expected_version);

                        if matches {
                            st.insert(key, types_kv::VersionedValue {
                                value: value.clone(),
                                version,
                            });
                            types_kv::Response::new(value, version)
                        } else {
                            types_kv::Response::none()
                        }
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
        // No entry applies while this runs, so the copy matches `last_applied_log_id`.
        let kvs = self.data.kvs.lock().await.clone();

        SnapshotBuilder {
            meta: SnapshotMeta {
                last_log_id: self.data.last_applied_log_id.clone(),
                last_membership: self.data.last_membership.clone(),
            },
            kvs,
            snapshots: self.snapshots.clone(),
        }
    }

    async fn install_snapshot(&mut self, meta: &SnapshotMeta, snapshot: SnapshotData) -> Result<(), io::Error> {
        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        self.update_state_machine_(new_snapshot.clone()).await?;

        self.snapshots.save(&new_snapshot)?;

        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot>, io::Error> {
        let x = self.snapshots.get()?;
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::time::SystemTime;

    use futures::stream;
    use openraft::RaftSnapshotBuilder;
    use openraft::entry::RaftEntry;
    use openraft::storage::RaftStateMachine;
    use openraft::testing::log_id;
    use types_kv::VersionedValue;

    use super::new_storage;
    use crate::TypeConfig;
    use crate::store::StateMachineStore;
    use crate::typ::*;

    /// A data directory removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
            Self(std::env::temp_dir().join(format!("openraft-jepsen-{name}-{}-{nanos}", std::process::id())))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn set(term: u64, index: u64, key: &str, value: &str) -> Entry {
        Entry::new_normal(
            log_id::<TypeConfig>(term, "n1".to_string(), index),
            types_kv::Request::Set {
                key: key.to_string(),
                value: value.to_string(),
            },
        )
    }

    async fn apply(sm: &mut StateMachineStore, entries: Vec<Entry>) {
        sm.apply(stream::iter(entries.into_iter().map(|entry| Ok((entry, None))))).await.unwrap();
    }

    fn kvs(snapshot: &Snapshot) -> BTreeMap<String, VersionedValue> {
        serde_json::from_slice(snapshot.snapshot.get_ref()).unwrap()
    }

    #[tokio::test]
    async fn builder_holds_the_state_it_was_requested_at() {
        let dir = TempDir::new("frozen");
        let (_log_store, mut sm) = new_storage(&dir.0).await;

        apply(&mut sm, vec![set(1, 1, "a", "1")]).await;
        let mut builder = sm.get_snapshot_builder().await;
        apply(&mut sm, vec![set(1, 2, "a", "2"), set(1, 3, "b", "3")]).await;

        let snapshot = builder.build_snapshot().await.unwrap();

        assert_eq!(
            Some(log_id::<TypeConfig>(1, "n1".to_string(), 1)),
            snapshot.meta.last_log_id
        );
        assert_eq!(
            BTreeMap::from([("a".to_string(), VersionedValue {
                value: "1".to_string(),
                version: 1,
            })]),
            kvs(&snapshot)
        );
    }

    #[tokio::test]
    async fn late_build_keeps_a_newer_installed_snapshot() {
        let dir = TempDir::new("install");
        let (_log_store, mut sm) = new_storage(&dir.0).await;

        apply(&mut sm, vec![set(1, 1, "a", "1")]).await;
        let mut builder = sm.get_snapshot_builder().await;

        let installed = BTreeMap::from([("a".to_string(), VersionedValue {
            value: "5".to_string(),
            version: 5,
        })]);
        let meta = SnapshotMeta {
            last_log_id: Some(log_id::<TypeConfig>(2, "n2".to_string(), 5)),
            last_membership: Default::default(),
        };
        sm.install_snapshot(&meta, Cursor::new(serde_json::to_vec(&installed).unwrap())).await.unwrap();

        builder.build_snapshot().await.unwrap();

        let stored = sm.get_current_snapshot().await.unwrap().unwrap();
        assert_eq!(meta.last_log_id, stored.meta.last_log_id);
        assert_eq!(installed, kvs(&stored));
    }
}
