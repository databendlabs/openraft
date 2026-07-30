use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use dir_transfer::DirFrameProducer;
use dir_transfer::DirWriter;
use dir_transfer::recv_dir;
use dir_transfer::send_dir;
use futures::stream;
use openraft::RaftSnapshotBuilder;
use openraft::entry::RaftEntry;
use openraft::storage::RaftStateMachine;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::EntryOf;
use rocksdb::ColumnFamilyDescriptor;
use rocksdb::Options;
use tempfile::TempDir;

use super::*;
use crate::TypeConfig;
use crate::state_machine::active_db::CURRENT_FILE;
use crate::state_machine::queue_transport::QueueTransport;

fn new_state_machine() -> (TempDir, RocksStateMachine<TypeConfig>) {
    let temp_dir = TempDir::new().unwrap();
    let state_machine = RocksStateMachine::<TypeConfig>::open(temp_dir.path()).unwrap();
    (temp_dir, state_machine)
}

async fn apply_set(state_machine: &mut RocksStateMachine<TypeConfig>, index: u64, key: &str, value: &str) {
    let entries: Vec<Result<EntryResponder<TypeConfig>, io::Error>> = vec![Ok((
        EntryOf::<TypeConfig>::new_normal(
            openraft::testing::log_id::<TypeConfig>(1, 1, index),
            types_kv::Request::set(key, value),
        ),
        None,
    ))];
    state_machine.apply(stream::iter(entries)).await.unwrap();
}

async fn build_snapshot(
    state_machine: &mut RocksStateMachine<TypeConfig>,
) -> SnapshotOf<TypeConfig, RocksSnapshotData> {
    state_machine.try_create_snapshot_builder(true).await.unwrap().build_snapshot().await.unwrap()
}

fn db_data(db: &DB) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let cf = column_family(db, "sm_data").unwrap();
    db.iterator_cf(cf, rocksdb::IteratorMode::Start)
        .map(|item| {
            let (key, value) = item.unwrap();
            (key.to_vec(), value.to_vec())
        })
        .collect()
}

fn state_data(state_machine: &RocksStateMachine<TypeConfig>) -> BTreeMap<Vec<u8>, Vec<u8>> {
    db_data(state_machine.active.db())
}

fn active_path(state_machine: &RocksStateMachine<TypeConfig>) -> PathBuf {
    state_machine.active.db_path().to_path_buf()
}

#[test]
fn cas_sees_earlier_write_in_same_batch() {
    TypeConfig::run(async {
        let (_temp_dir, mut state_machine) = new_state_machine();
        apply_set(&mut state_machine, 1, "key", "A").await;

        // Both entries expect version 1, the index of the `Set`; only the first one succeeds.
        let entries: Vec<Result<EntryResponder<TypeConfig>, io::Error>> = vec![
            Ok((
                EntryOf::<TypeConfig>::new_normal(
                    openraft::testing::log_id::<TypeConfig>(1, 1, 2),
                    types_kv::Request::compare_and_set("key", 1, "B"),
                ),
                None,
            )),
            Ok((
                EntryOf::<TypeConfig>::new_normal(
                    openraft::testing::log_id::<TypeConfig>(1, 1, 3),
                    types_kv::Request::compare_and_set("key", 1, "C"),
                ),
                None,
            )),
        ];
        state_machine.apply(stream::iter(entries)).await.unwrap();

        let db = state_machine.active.db();
        let cf = column_family(db, "sm_data").unwrap();
        let bytes = db.get_cf(cf, "key").unwrap().unwrap();
        let value: types_kv::VersionedValue = deserialize(&bytes).unwrap();
        assert_eq!(
            types_kv::VersionedValue {
                value: "B".to_string(),
                version: 2,
            },
            value
        );
    });
}

#[test]
fn builder_captures_stable_checkpoint_and_publishes_it() {
    TypeConfig::run(async {
        let (_temp_dir, mut state_machine) = new_state_machine();
        apply_set(&mut state_machine, 1, "key", "old").await;
        let expected_checkpoint_data = state_data(&state_machine);
        let mut builder = state_machine.try_create_snapshot_builder(true).await.unwrap();
        apply_set(&mut state_machine, 2, "key", "new").await;

        let snapshot = builder.build_snapshot().await.unwrap();
        let checkpoint = snapshot.snapshot.open_read_only().unwrap();
        let checkpoint_meta = read_meta::<TypeConfig>(&checkpoint).unwrap();
        assert_eq!(snapshot.meta, checkpoint_meta);
        assert_eq!(expected_checkpoint_data, db_data(&checkpoint));

        let current = state_machine.get_current_snapshot().await.unwrap().unwrap();
        assert_eq!(snapshot.meta, current.meta);
        assert_eq!(snapshot.snapshot.db_path(), current.snapshot.db_path());
    });
}

#[test]
fn active_database_and_snapshot_are_peer_generations() {
    TypeConfig::run(async {
        let (base_dir, mut state_machine) = new_state_machine();
        let snapshot = build_snapshot(&mut state_machine).await;

        assert_eq!(base_dir.path(), active_path(&state_machine).parent().unwrap());
        assert_eq!(base_dir.path(), snapshot.snapshot.db_path().parent().unwrap());
        assert_ne!(active_path(&state_machine), snapshot.snapshot.db_path());
    });
}

#[test]
fn older_builder_does_not_replace_newer_snapshot() {
    TypeConfig::run(async {
        let (_temp_dir, mut state_machine) = new_state_machine();
        apply_set(&mut state_machine, 1, "key", "value").await;
        let mut older = state_machine.try_create_snapshot_builder(true).await.unwrap();
        let mut newer = state_machine.try_create_snapshot_builder(true).await.unwrap();

        let newer_snapshot = newer.build_snapshot().await.unwrap();
        let older_snapshot = older.build_snapshot().await.unwrap();
        assert_eq!(newer_snapshot.meta.last_log_id, older_snapshot.meta.last_log_id);

        let current = state_machine.get_current_snapshot().await.unwrap().unwrap();
        assert_eq!(newer_snapshot.snapshot.db_path(), current.snapshot.db_path());
    });
}

#[test]
fn older_builder_does_not_replace_installed_snapshot() {
    TypeConfig::run(async {
        let (_target_dir, mut target) = new_state_machine();
        apply_set(&mut target, 1, "key", "old").await;
        let mut older = target.try_create_snapshot_builder(true).await.unwrap();

        let (_source_dir, mut source) = new_state_machine();
        apply_set(&mut source, 2, "key", "installed").await;
        let installed = build_snapshot(&mut source).await;
        target.install_snapshot(&installed.meta, installed.snapshot.clone()).await.unwrap();
        older.build_snapshot().await.unwrap();

        assert_eq!(state_data(&source), state_data(&target));
        let current = target.get_current_snapshot().await.unwrap().unwrap();
        assert_eq!(installed.meta, current.meta);
        assert_eq!(installed.snapshot.db_path(), current.snapshot.db_path());
    });
}

#[test]
fn install_replaces_data_metadata_and_current_snapshot() {
    TypeConfig::run(async {
        let (_source_dir, mut source) = new_state_machine();
        apply_set(&mut source, 1, "a", "one").await;
        apply_set(&mut source, 2, "b", "two").await;
        let snapshot = build_snapshot(&mut source).await;

        let (_target_dir, mut target) = new_state_machine();
        apply_set(&mut target, 1, "a", "stale").await;
        apply_set(&mut target, 2, "extra", "remove").await;
        let previous_active = active_path(&target);
        target.install_snapshot(&snapshot.meta, snapshot.snapshot.clone()).await.unwrap();

        assert_eq!(state_data(&source), state_data(&target));
        assert_eq!(source.get_meta().unwrap(), target.get_meta().unwrap());
        assert_ne!(previous_active, active_path(&target));
        assert_ne!(snapshot.snapshot.db_path(), active_path(&target));
        assert!(!previous_active.exists());
        assert_eq!(
            active_path(&target).file_name().unwrap().to_str().unwrap(),
            fs::read_to_string(target.base_dir.join(CURRENT_FILE)).unwrap().trim()
        );
        let current = target.get_current_snapshot().await.unwrap().unwrap();
        assert_eq!(snapshot.meta, current.meta);
        assert_eq!(snapshot.snapshot.db_path(), current.snapshot.db_path());
    });
}

#[test]
fn install_removes_existing_last_log_id_when_snapshot_has_none() {
    TypeConfig::run(async {
        let (_source_dir, mut source) = new_state_machine();
        let snapshot = build_snapshot(&mut source).await;
        assert_eq!(None, snapshot.meta.last_log_id);

        let (_target_dir, mut target) = new_state_machine();
        apply_set(&mut target, 1, "old", "value").await;
        target.install_snapshot(&snapshot.meta, snapshot.snapshot).await.unwrap();

        assert_eq!(snapshot.meta, target.get_meta().unwrap());
        assert_eq!(BTreeMap::new(), state_data(&target));
    });
}

#[test]
fn checkpoint_transfer_installs_full_state() {
    TypeConfig::run(async {
        let (_source_dir, mut source) = new_state_machine();
        apply_set(&mut source, 1, "a", "one").await;
        apply_set(&mut source, 2, "b", "two").await;
        let snapshot = build_snapshot(&mut source).await;

        let (_target_dir, mut target) = new_state_machine();
        let received = RocksSnapshotData::new_empty(&target.base_dir).unwrap();
        let received_path = received.db_path().to_path_buf();
        let producer = DirFrameProducer::new(snapshot.snapshot.db_path(), 1024).unwrap();
        let mut transport = QueueTransport::default();
        send_dir(producer, &mut transport).await.unwrap();
        recv_dir(&mut transport, DirWriter::new(received.db_path().to_path_buf())).await.unwrap();
        target.install_snapshot(&snapshot.meta, received).await.unwrap();

        assert_eq!(state_data(&source), state_data(&target));
        assert_eq!(source.get_meta().unwrap(), target.get_meta().unwrap());
        assert_eq!(target.base_dir.as_path(), received_path.parent().unwrap());
        assert!(received_path.exists());
        assert!(transport.is_empty());
    });
}

#[test]
fn restart_opens_generation_named_by_current() {
    TypeConfig::run(async {
        let base_dir = TempDir::new().unwrap();
        let mut state_machine = RocksStateMachine::<TypeConfig>::open(base_dir.path()).unwrap();
        apply_set(&mut state_machine, 1, "key", "value").await;
        let expected_data = state_data(&state_machine);
        let expected_meta = state_machine.get_meta().unwrap();
        let expected_path = active_path(&state_machine);
        drop(state_machine);

        let mut reopened = RocksStateMachine::<TypeConfig>::open(base_dir.path()).unwrap();
        assert_eq!(expected_path, active_path(&reopened));
        assert_eq!(expected_data, state_data(&reopened));
        assert_eq!(expected_meta, reopened.get_meta().unwrap());
        assert!(reopened.get_current_snapshot().await.unwrap().is_none());
    });
}

#[test]
fn startup_rejects_missing_current_generation_without_cleanup() {
    TypeConfig::run(async {
        let base_dir = TempDir::new().unwrap();
        let mut state_machine = RocksStateMachine::<TypeConfig>::open(base_dir.path()).unwrap();
        apply_set(&mut state_machine, 1, "key", "value").await;
        let expected_data = state_data(&state_machine);
        let existing_path = active_path(&state_machine);
        let existing_name = existing_path.file_name().unwrap();
        let existing_name = existing_name.to_str().unwrap();
        let missing_name = "openraft-db-missing";
        drop(state_machine);
        fs::write(base_dir.path().join(CURRENT_FILE), missing_name).unwrap();

        RocksStateMachine::<TypeConfig>::open(base_dir.path()).unwrap_err();

        assert!(existing_path.exists());

        fs::write(base_dir.path().join(CURRENT_FILE), existing_name).unwrap();
        let recovered = RocksStateMachine::<TypeConfig>::open(base_dir.path()).unwrap();
        assert_eq!(expected_data, state_data(&recovered));
    });
}

#[test]
fn install_rejects_checkpoint_without_data_column_family() {
    TypeConfig::run(async {
        let (_target_dir, mut target) = new_state_machine();
        apply_set(&mut target, 1, "key", "value").await;
        let data_before = state_data(&target);
        let meta_before = target.get_meta().unwrap();
        let path_before = active_path(&target);

        let received = RocksSnapshotData::new_empty(&target.base_dir).unwrap();
        let received_path = received.db_path().to_path_buf();
        let mut options = Options::default();
        options.create_if_missing(true);
        options.create_missing_column_families(true);
        let descriptors = [ColumnFamilyDescriptor::new("sm_meta", Options::default())];
        let malformed = DB::open_cf_descriptors(&options, &received_path, descriptors).unwrap();
        drop(malformed);
        let meta = SnapshotMetaOf::<TypeConfig> {
            last_log_id: None,
            last_membership: StoredMembershipOf::<TypeConfig>::default(),
        };

        target.install_snapshot(&meta, received).await.unwrap_err();

        assert_eq!(data_before, state_data(&target));
        assert_eq!(meta_before, target.get_meta().unwrap());
        assert_eq!(path_before, active_path(&target));
        assert!(!received_path.exists());
    });
}

#[test]
fn active_database_contains_only_state_machine_column_families() {
    let (_base_dir, state_machine) = new_state_machine();
    let mut column_families = DB::list_cf(&Options::default(), active_path(&state_machine)).unwrap();
    column_families.sort();

    assert_eq!(vec!["default", "sm_data", "sm_meta"], column_families);
}

#[test]
fn receiving_dir_lives_until_last_clone_drops() {
    let base_dir = TempDir::new().unwrap();
    let snapshot = RocksSnapshotData::new_empty(base_dir.path()).unwrap();
    let received_path = snapshot.db_path().to_path_buf();
    let clone = snapshot.clone();

    drop(snapshot);
    assert!(received_path.exists());
    drop(clone);
    assert!(!received_path.exists());
}

#[test]
fn failed_transfer_removes_dir_when_snapshot_drops() {
    TypeConfig::run(async {
        let base_dir = TempDir::new().unwrap();
        let snapshot = RocksSnapshotData::new_empty(base_dir.path()).unwrap();
        let received_path = snapshot.db_path().to_path_buf();
        let mut transport = QueueTransport::default();

        let error = recv_dir(&mut transport, DirWriter::new(snapshot.db_path().to_path_buf())).await.unwrap_err();
        assert_eq!(io::ErrorKind::UnexpectedEof, error.kind());
        drop(snapshot);
        assert!(!received_path.exists());
    });
}

#[test]
fn startup_removes_only_prefixed_orphan_generations() {
    let base_dir = TempDir::new().unwrap();
    let orphan = base_dir.path().join("openraft-db-orphan");
    let unrelated = base_dir.path().join("keep-me");
    fs::create_dir(&orphan).unwrap();
    fs::create_dir(&unrelated).unwrap();

    let _state_machine = RocksStateMachine::<TypeConfig>::open(base_dir.path()).unwrap();
    assert!(!orphan.exists());
    assert!(unrelated.exists());
}

#[cfg(unix)]
#[test]
fn checkpoint_hard_links_sst_files() {
    use std::os::unix::fs::MetadataExt;

    TypeConfig::run(async {
        let (_temp_dir, mut state_machine) = new_state_machine();
        apply_set(&mut state_machine, 1, "key", "value").await;
        state_machine.active.db().flush().unwrap();
        let snapshot = build_snapshot(&mut state_machine).await;
        let live_sst = first_sst(state_machine.active.db_path());
        let checkpoint_sst = snapshot.snapshot.db_path().join(live_sst.file_name().unwrap());

        assert_eq!(
            fs::metadata(live_sst).unwrap().ino(),
            fs::metadata(checkpoint_sst).unwrap().ino()
        );
    });
}

#[cfg(unix)]
fn first_sst(db_path: &Path) -> PathBuf {
    fs::read_dir(db_path)
        .unwrap()
        .map(Result::unwrap)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "sst"))
        .unwrap()
}
