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

use crate::SNAPSHOT_INSTALLED_MARKER;
use crate::TypeConfig;
use crate::request::Request;
use crate::typ::*;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StoredSnapshot {
    pub meta: SnapshotMeta,

    /// The data of the state machine at the time of this snapshot, a serialized
    /// [`SnapshotPayload`].
    pub data: Vec<u8>,
}

/// The state machine data a snapshot carries besides its meta.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotPayload {
    /// The [`AppliedState::applied_chain`] at the snapshot's last log id.
    pub applied_chain: u64,

    pub kvs: BTreeMap<String, types_kv::VersionedValue>,

    #[serde(default)]
    pub lists: BTreeMap<String, Vec<String>>,
}

/// The applied state, behind one lock so that the snapshot builder and the consistency endpoint
/// each read `last_applied`, the membership and the data of a single instant.
#[derive(Debug, Clone, Default)]
pub struct AppliedState {
    pub last_applied: Option<LogId>,

    pub last_membership: StoredMembership,

    /// Rolling digest of every entry applied so far, carried in snapshots.
    ///
    /// Two replicas at the same `last_applied` agree on it only if they applied the same log, so
    /// comparing it compares every applied log id and payload, including the ones a snapshot has
    /// since covered.
    pub applied_chain: u64,

    pub kvs: BTreeMap<String, types_kv::VersionedValue>,

    /// Lists built by `Append`, in apply order.
    pub lists: BTreeMap<String, Vec<String>>,
}

pub type SharedState = Arc<Mutex<AppliedState>>;

impl AppliedState {
    fn apply(&mut self, entry: Entry) -> Result<types_kv::Response, io::Error> {
        self.applied_chain = chain(self.applied_chain, &entry)?;
        self.last_applied = Some(entry.log_id.clone());

        let version = entry.log_id.index();

        let response = match entry.payload {
            EntryPayload::Blank => types_kv::Response::none(),
            EntryPayload::Normal(req) => match req {
                Request::Set { key, value } => {
                    self.kvs.insert(key, types_kv::VersionedValue {
                        value: value.clone(),
                        version,
                    });
                    types_kv::Response::new(value, version)
                }
                Request::CompareAndSet {
                    key,
                    expected_version,
                    value,
                } => {
                    let matches = self.kvs.get(&key).is_some_and(|current| current.version == expected_version);

                    if matches {
                        self.kvs.insert(key, types_kv::VersionedValue {
                            value: value.clone(),
                            version,
                        });
                        types_kv::Response::new(value, version)
                    } else {
                        types_kv::Response::none()
                    }
                }
                Request::Append { key, value } => {
                    self.lists.entry(key).or_default().push(value.clone());
                    types_kv::Response::new(value, version)
                }
            },
            EntryPayload::Membership(mem) => {
                self.last_membership = StoredMembership::new(Some(entry.log_id), mem);
                types_kv::Response::none()
            }
        };

        Ok(response)
    }

    fn snapshot_meta(&self) -> SnapshotMeta {
        SnapshotMeta {
            last_log_id: self.last_applied.clone(),
            last_membership: self.last_membership.clone(),
        }
    }

    fn snapshot_payload(&self) -> SnapshotPayload {
        SnapshotPayload {
            applied_chain: self.applied_chain,
            kvs: self.kvs.clone(),
            lists: self.lists.clone(),
        }
    }

    fn restore(&mut self, snapshot: &StoredSnapshot) -> Result<(), io::Error> {
        let payload: SnapshotPayload = serde_json::from_slice(&snapshot.data).map_err(invalid_data)?;

        self.last_applied = snapshot.meta.last_log_id.clone();
        self.last_membership = snapshot.meta.last_membership.clone();
        self.applied_chain = payload.applied_chain;
        self.kvs = payload.kvs;
        self.lists = payload.lists;
        Ok(())
    }

    /// Digest of the registers, the lists and the membership.
    pub fn state_digest(&self) -> Result<u64, io::Error> {
        let bytes = serde_json::to_vec(&(&self.last_membership, &self.kvs, &self.lists)).map_err(invalid_data)?;
        Ok(fnv1a(FNV_OFFSET, &bytes))
    }
}

/// Folds one applied entry, its log id and payload, into the digest of the applied log.
fn chain(prev: u64, entry: &Entry) -> Result<u64, io::Error> {
    let bytes = serde_json::to_vec(entry).map_err(invalid_data)?;
    Ok(fnv1a(fnv1a(FNV_OFFSET, &prev.to_le_bytes()), &bytes))
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// 64-bit FNV-1a, which unlike `DefaultHasher` is specified, so every build agrees on it.
fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn invalid_data(e: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

#[derive(Debug)]
pub struct StateMachineStore {
    pub state: SharedState,

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

/// A frozen copy of the state machine, taken when the build was requested.
///
/// OpenRaft runs `build_snapshot` in a spawned task while later entries keep applying, so the
/// builder must not share the live map: its data could then be newer than `meta.last_log_id`.
pub struct SnapshotBuilder {
    meta: SnapshotMeta,
    payload: SnapshotPayload,
    snapshots: SnapshotStore,
}

impl RaftSnapshotBuilder<TypeConfig> for SnapshotBuilder {
    type SnapshotData = Cursor<Vec<u8>>;

    async fn build_snapshot(&mut self) -> Result<Snapshot, io::Error> {
        let snapshot = StoredSnapshot {
            meta: self.meta.clone(),
            data: serde_json::to_vec(&self.payload).map_err(invalid_data)?,
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
        let snapshots = SnapshotStore {
            db,
            lock: Arc::new(std::sync::Mutex::new(())),
        };

        let mut state = AppliedState::default();
        if let Some(snap) = snapshots.get()? {
            state.restore(&snap)?;
        }

        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            snapshots,
        })
    }
}

impl RaftStateMachine<TypeConfig> for StateMachineStore {
    type SnapshotData = Cursor<Vec<u8>>;

    type SnapshotBuilder = SnapshotBuilder;

    async fn applied_state(&mut self) -> Result<(Option<LogId>, StoredMembership), io::Error> {
        let state = self.state.lock().await;
        Ok((state.last_applied.clone(), state.last_membership.clone()))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend {
        while let Some((entry, responder)) = entries.try_next().await? {
            let response = self.state.lock().await.apply(entry)?;

            if let Some(responder) = responder {
                responder.send(response);
            }
        }
        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        let state = self.state.lock().await;

        SnapshotBuilder {
            meta: state.snapshot_meta(),
            payload: state.snapshot_payload(),
            snapshots: self.snapshots.clone(),
        }
    }

    async fn install_snapshot(&mut self, meta: &SnapshotMeta, snapshot: SnapshotData) -> Result<(), io::Error> {
        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        self.state.lock().await.restore(&new_snapshot)?;

        self.snapshots.save(&new_snapshot)?;

        // Startup restores the stored snapshot in `StateMachineStore::new`, so OpenRaft never
        // calls this to restore one and every marker is an install from a leader.
        eprintln!("{SNAPSHOT_INSTALLED_MARKER} last_log_id={:?}", meta.last_log_id);

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
    // Keep each write in RocksDB's in-process WAL buffer until a `flush_wal`, so a SIGKILL drops
    // what was written but never flushed, as a power loss would. Everything the stores report
    // durable is flushed first: log appends before their IO callback, votes and snapshots before
    // returning. Truncation and purge are not, which OpenRaft allows.
    db_opts.set_manual_wal_flush(true);

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

    use super::SnapshotPayload;
    use super::new_storage;
    use crate::TypeConfig;
    use crate::request::Request;
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
        Entry::new_normal(log_id::<TypeConfig>(term, "n1".to_string(), index), Request::Set {
            key: key.to_string(),
            value: value.to_string(),
        })
    }

    fn append(term: u64, index: u64, key: &str, value: &str) -> Entry {
        Entry::new_normal(log_id::<TypeConfig>(term, "n1".to_string(), index), Request::Append {
            key: key.to_string(),
            value: value.to_string(),
        })
    }

    async fn apply(sm: &mut StateMachineStore, entries: Vec<Entry>) {
        sm.apply(stream::iter(entries.into_iter().map(|entry| Ok((entry, None))))).await.unwrap();
    }

    fn payload(snapshot: &Snapshot) -> SnapshotPayload {
        serde_json::from_slice(snapshot.snapshot.get_ref()).unwrap()
    }

    /// The applied log id, chain and state digest a replica reports for comparison.
    async fn fingerprint(sm: &StateMachineStore) -> (Option<LogId>, u64, u64) {
        let state = sm.state.lock().await;
        (
            state.last_applied.clone(),
            state.applied_chain,
            state.state_digest().unwrap(),
        )
    }

    #[tokio::test]
    async fn builder_holds_the_state_it_was_requested_at() {
        let dir = TempDir::new("frozen");
        let (_log_store, mut sm) = new_storage(&dir.0).await;

        apply(&mut sm, vec![set(1, 1, "a", "1")]).await;
        let chain_at_request = sm.state.lock().await.applied_chain;
        let mut builder = sm.get_snapshot_builder().await;
        apply(&mut sm, vec![set(1, 2, "a", "2"), set(1, 3, "b", "3")]).await;

        let snapshot = builder.build_snapshot().await.unwrap();

        assert_eq!(
            Some(log_id::<TypeConfig>(1, "n1".to_string(), 1)),
            snapshot.meta.last_log_id
        );
        assert_eq!(
            SnapshotPayload {
                applied_chain: chain_at_request,
                kvs: BTreeMap::from([("a".to_string(), VersionedValue {
                    value: "1".to_string(),
                    version: 1,
                })]),
                lists: BTreeMap::new(),
            },
            payload(&snapshot)
        );
    }

    #[tokio::test]
    async fn late_build_keeps_a_newer_installed_snapshot() {
        let dir = TempDir::new("install");
        let (_log_store, mut sm) = new_storage(&dir.0).await;

        apply(&mut sm, vec![set(1, 1, "a", "1")]).await;
        let mut builder = sm.get_snapshot_builder().await;

        let installed = SnapshotPayload {
            applied_chain: 7,
            kvs: BTreeMap::from([("a".to_string(), VersionedValue {
                value: "5".to_string(),
                version: 5,
            })]),
            lists: BTreeMap::new(),
        };
        let meta = SnapshotMeta {
            last_log_id: Some(log_id::<TypeConfig>(2, "n2".to_string(), 5)),
            last_membership: Default::default(),
        };
        sm.install_snapshot(&meta, Cursor::new(serde_json::to_vec(&installed).unwrap())).await.unwrap();

        builder.build_snapshot().await.unwrap();

        let stored = sm.get_current_snapshot().await.unwrap().unwrap();
        assert_eq!(meta.last_log_id, stored.meta.last_log_id);
        assert_eq!(installed, payload(&stored));
    }

    #[tokio::test]
    async fn a_snapshot_carries_the_applied_chain() {
        let dirs = [
            TempDir::new("chain-a"),
            TempDir::new("chain-b"),
            TempDir::new("chain-c"),
        ];
        let (_, mut replayed) = new_storage(&dirs[0].0).await;
        let (_, mut installed) = new_storage(&dirs[1].0).await;
        let (_, mut diverged) = new_storage(&dirs[2].0).await;

        apply(&mut replayed, vec![set(1, 1, "a", "1"), set(1, 2, "b", "2")]).await;
        let snapshot = replayed.get_snapshot_builder().await.build_snapshot().await.unwrap();
        installed.install_snapshot(&snapshot.meta, snapshot.snapshot).await.unwrap();

        // Same final state, reached through a different entry at index 1.
        apply(&mut diverged, vec![set(1, 1, "a", "0"), set(1, 2, "b", "2")]).await;
        diverged.state.lock().await.kvs.get_mut("a").unwrap().value = "1".to_string();

        for sm in [&mut replayed, &mut installed, &mut diverged] {
            apply(sm, vec![set(1, 3, "c", "3")]).await;
        }

        let replayed = fingerprint(&replayed).await;
        assert_eq!(replayed, fingerprint(&installed).await);

        let diverged = fingerprint(&diverged).await;
        assert_eq!(replayed.0, diverged.0);
        assert_eq!(replayed.2, diverged.2, "the state alone does not show the divergence");
        assert_ne!(replayed.1, diverged.1, "the applied chain does");
    }

    #[tokio::test]
    async fn an_unflushed_write_is_lost_on_a_crash() {
        let dir = TempDir::new("manual-flush");
        let crashed = TempDir::new("manual-flush-crash");

        let db = {
            let (log_store, sm) = new_storage(&dir.0).await;
            drop(log_store);
            sm.snapshots.db.clone()
        };
        let cf = db.cf_handle("logs").unwrap();
        db.put_cf(cf, b"flushed", b"1").unwrap();
        db.flush_wal(true).unwrap();
        db.put_cf(cf, b"unflushed", b"1").unwrap();
        assert!(
            db.get_cf(cf, b"unflushed").unwrap().is_some(),
            "readable before the flush"
        );

        // A copy of the files as they are on disk is what a SIGKILL leaves behind.
        std::fs::create_dir_all(&crashed.0).unwrap();
        for file in std::fs::read_dir(&dir.0).unwrap() {
            let file = file.unwrap();
            std::fs::copy(file.path(), crashed.0.join(file.file_name())).unwrap();
        }

        let (log_store, sm) = new_storage(&crashed.0).await;
        drop(log_store);
        let db = sm.snapshots.db.clone();
        let cf = db.cf_handle("logs").unwrap();
        assert!(db.get_cf(cf, b"flushed").unwrap().is_some());
        assert!(db.get_cf(cf, b"unflushed").unwrap().is_none());
    }

    #[tokio::test]
    async fn appends_survive_a_snapshot_install_in_order() {
        let dirs = [TempDir::new("append-a"), TempDir::new("append-b")];
        let (_, mut source) = new_storage(&dirs[0].0).await;
        let (_, mut installed) = new_storage(&dirs[1].0).await;

        apply(&mut source, vec![append(1, 1, "l", "1"), append(1, 2, "l", "2")]).await;
        let snapshot = source.get_snapshot_builder().await.build_snapshot().await.unwrap();
        installed.install_snapshot(&snapshot.meta, snapshot.snapshot).await.unwrap();
        apply(&mut installed, vec![append(1, 3, "l", "3")]).await;

        assert_eq!(
            Some(&vec!["1".to_string(), "2".to_string(), "3".to_string()]),
            installed.state.lock().await.lists.get("l")
        );
    }

    #[tokio::test]
    async fn restart_restores_the_applied_chain() {
        let dir = TempDir::new("restart");
        let expected = {
            let (_log_store, mut sm) = new_storage(&dir.0).await;
            apply(&mut sm, vec![set(1, 1, "a", "1"), set(1, 2, "b", "2")]).await;
            sm.get_snapshot_builder().await.build_snapshot().await.unwrap();
            fingerprint(&sm).await
        };

        let (_log_store, sm) = new_storage(&dir.0).await;
        assert_eq!(expected, fingerprint(&sm).await);
    }
}
