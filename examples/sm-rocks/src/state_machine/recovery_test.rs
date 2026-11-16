use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use futures::stream;
use log_rocks::RocksLogStore;
use openraft::RaftSnapshotBuilder;
use openraft::StorageHelper;
use openraft::alias::EntryOf;
use openraft::alias::MembershipOf;
use openraft::entry::RaftEntry;
use openraft::impls::Membership;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftLogStorageExt;
use openraft::testing::log_id;
use tempfile::TempDir;
use types_kv::Request;
use types_kv::VersionedValue;

use super::*;
use crate::TypeConfig;
use crate::snapshot::GENERATION_DIR_PREFIX;
use crate::state_machine::active_db::CURRENT_FILE;
use crate::state_machine::fault::Failure;

mod crash_test;

fn temporary_root() -> PathBuf {
    let root = TempDir::new().unwrap().keep();
    fs::canonicalize(root).unwrap()
}

fn entries(installed: bool) -> Vec<EntryOf<TypeConfig>> {
    let mut entries = vec![
        EntryOf::<TypeConfig>::new_membership(log_id::<TypeConfig>(1, 1, 1), membership(false)),
        EntryOf::<TypeConfig>::new_normal(log_id::<TypeConfig>(1, 1, 2), Request::set("key", "old")),
    ];
    if installed {
        entries.extend([
            EntryOf::<TypeConfig>::new_membership(log_id::<TypeConfig>(1, 1, 3), membership(true)),
            EntryOf::<TypeConfig>::new_normal(log_id::<TypeConfig>(1, 1, 4), Request::set("key", "new")),
            EntryOf::<TypeConfig>::new_normal(log_id::<TypeConfig>(1, 1, 5), Request::set("extra", "value")),
        ]);
    }
    entries
}

fn membership(installed: bool) -> MembershipOf<TypeConfig> {
    let nodes = if installed {
        BTreeSet::from([1, 2])
    } else {
        BTreeSet::from([1])
    };
    Membership::new_with_defaults(vec![nodes], [])
}

fn expected_meta(installed: bool) -> SnapshotMetaOf<TypeConfig> {
    let index = if installed { 5 } else { 2 };
    let membership_index = if installed { 3 } else { 1 };
    SnapshotMetaOf::<TypeConfig> {
        last_log_id: Some(log_id::<TypeConfig>(1, 1, index)),
        last_membership: StoredMembershipOf::<TypeConfig>::new(
            Some(log_id::<TypeConfig>(1, 1, membership_index)),
            membership(installed),
        ),
    }
}

fn expected_data(installed: bool) -> BTreeMap<String, VersionedValue> {
    let values = if installed {
        vec![("key", "new", 4), ("extra", "value", 5)]
    } else {
        vec![("key", "old", 2)]
    };
    values
        .into_iter()
        .map(|(key, value, version)| {
            (key.to_string(), VersionedValue {
                value: value.to_string(),
                version,
            })
        })
        .collect()
}

async fn apply_entries(state: &mut RocksStateMachine<TypeConfig>, installed: bool) -> io::Result<()> {
    let entries = entries(installed).into_iter();
    let entries = if installed { entries.skip(2) } else { entries.skip(0) };
    let items = entries.map(|entry| Ok((entry, None)));
    state.apply(stream::iter(items)).await
}

async fn prepare(root: &Path) -> (RocksStateMachine<TypeConfig>, SnapshotOf<TypeConfig, RocksSnapshotData>) {
    let mut source = RocksStateMachine::<TypeConfig>::open(root.join("source")).unwrap();
    apply_entries(&mut source, false).await.unwrap();
    apply_entries(&mut source, true).await.unwrap();
    let mut builder = source.get_snapshot_builder().await;
    let snapshot = builder.build_snapshot().await.unwrap();
    let mut target = RocksStateMachine::<TypeConfig>::open(root.join("target")).unwrap();
    apply_entries(&mut target, false).await.unwrap();
    target.get_snapshot_builder().await;
    (target, snapshot)
}

fn assert_state(state: &RocksStateMachine<TypeConfig>, installed: bool) {
    let data = state.data().unwrap();
    let meta = state.get_meta().unwrap();
    let expected_data = expected_data(installed);
    let expected_meta = expected_meta(installed);
    assert_eq!(expected_data, data);
    assert_eq!(expected_meta, meta);
}

fn generations(base: &Path) -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::new();
    for entry in fs::read_dir(base).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_string_lossy().starts_with(GENERATION_DIR_PREFIX) {
            paths.insert(entry.path());
        }
    }
    paths
}

async fn recover(root: &Path, installed: bool) {
    let mut state = RocksStateMachine::<TypeConfig>::open(root.join("target")).unwrap();
    assert_state(&state, installed);
    let current = state.get_current_snapshot().await.unwrap();
    assert!(current.is_none());
    let mut logs = RocksLogStore::<TypeConfig>::open(root.join("log")).unwrap();
    let meta = expected_meta(installed);
    logs.purge(meta.last_log_id.unwrap()).await.unwrap();
    let _initial = StorageHelper::new(&mut logs, &mut state).with_id(1).get_initial_state().await.unwrap();
    let snapshot = state.get_current_snapshot().await.unwrap().unwrap();
    assert_eq!(meta, snapshot.meta);
    let checkpoint = snapshot.snapshot.open_read_only().unwrap();
    let stored = read_meta::<TypeConfig>(&checkpoint).unwrap();
    assert_eq!(meta, stored);
    let data = checkpoint_data(&checkpoint);
    let expected = expected_data(installed);
    assert_eq!(expected, data);
    assert_state(&state, installed);
}

fn checkpoint_data(db: &DB) -> BTreeMap<String, VersionedValue> {
    let column = column_family(db, "sm_data").unwrap();
    let mut data = BTreeMap::new();
    for item in db.iterator_cf(column, rocksdb::IteratorMode::Start) {
        let (key, value) = item.unwrap();
        let key = String::from_utf8(key.to_vec()).unwrap();
        let value = deserialize(&value).unwrap();
        data.insert(key, value);
    }
    data
}

#[test]
fn mismatched_snapshot_metadata_preserves_complete_previous_state() {
    TypeConfig::run(async {
        for mismatch in ["last_log_id", "membership"] {
            let root = temporary_root();
            let (mut target, snapshot) = prepare(&root).await;
            let before = target.get_current_snapshot().await.unwrap().unwrap();
            let directories = generations(&target.base_dir);
            let current = fs::read(target.base_dir.join(CURRENT_FILE)).unwrap();
            let mut metadata = snapshot.meta.clone();
            if mismatch == "last_log_id" {
                metadata.last_log_id = Some(log_id::<TypeConfig>(1, 1, 6));
            } else {
                metadata.last_membership = expected_meta(false).last_membership;
            }
            tracing::info!(mismatch, "Reject metadata that disagrees with the received database");
            {
                let error = target.install_snapshot(&metadata, snapshot.snapshot).await.unwrap_err();
                assert_eq!(io::ErrorKind::InvalidData, error.kind());
                assert_state(&target, false);
                let after = target.get_current_snapshot().await.unwrap().unwrap();
                assert_eq!(before.meta, after.meta);
                assert_eq!(before.snapshot.db_path(), after.snapshot.db_path());
                let current_after = fs::read(target.base_dir.join(CURRENT_FILE)).unwrap();
                let directories_after = generations(&target.base_dir);
                assert_eq!(current, current_after);
                assert_eq!(directories, directories_after);
            }
            drop(before);
            drop(target);
            recover(&root, false).await;
            fs::remove_dir_all(root).unwrap();
        }
    });
}

#[test]
fn publication_errors_keep_every_possible_current_generation() {
    TypeConfig::run(async {
        for point in [
            "candidate_prepared",
            "before_current_replace",
            "before_current_sync",
            "after_current_sync",
        ] {
            let root = temporary_root();
            let (mut target, snapshot) = prepare(&root).await;
            let previous_path = target.active.db_path().to_path_buf();
            let before = target.get_current_snapshot().await.unwrap().unwrap();
            tracing::info!(point, "Unwind publication and retain every possible recovery target");
            {
                let failure = Failure::new(&target.base_dir, point);
                let error = target.install_snapshot(&snapshot.meta, snapshot.snapshot).await.unwrap_err();
                let expected_error = format!("injected failure: {point}");
                let actual_error = error.to_string();
                assert_eq!(expected_error, actual_error);
                assert_state(&target, false);
                let after = target.get_current_snapshot().await.unwrap().unwrap();
                assert_eq!(before.meta, after.meta);
                assert_eq!(before.snapshot.db_path(), after.snapshot.db_path());
                drop(failure);
            }
            let selected = fs::read_to_string(target.base_dir.join(CURRENT_FILE)).unwrap();
            let selected = target.base_dir.join(selected.trim());
            let published = point == "before_current_sync" || point == "after_current_sync";
            drop(before);
            drop(target);
            let paths = generations(&root.join("target"));
            assert!(paths.contains(&previous_path));
            assert!(paths.contains(&selected));
            if published {
                assert_ne!(previous_path, selected);
                let expected_paths = BTreeSet::from([previous_path, selected]);
                assert_eq!(expected_paths, paths);
            }
            recover(&root, published).await;
            fs::remove_dir_all(root).unwrap();
        }
    });
}

#[test]
fn apply_errors_recover_a_complete_prefix() {
    TypeConfig::run(async {
        for point in ["before_apply_commit", "after_apply_commit"] {
            let root = temporary_root();
            let (mut target, snapshot) = prepare(&root).await;
            let failure = Failure::new(&target.base_dir, point);
            let error = apply_entries(&mut target, true).await.unwrap_err();
            let expected_error = format!("injected failure: {point}");
            let actual_error = error.to_string();
            assert_eq!(expected_error, actual_error);
            let committed = point == "after_apply_commit";
            assert_state(&target, committed);
            drop(failure);
            drop(snapshot);
            drop(target);
            recover(&root, committed).await;
            fs::remove_dir_all(root).unwrap();
        }
    });
}

#[test]
fn cleanup_error_keeps_selected_generation_and_startup_retries() {
    TypeConfig::run(async {
        let root = temporary_root();
        let (mut target, snapshot) = prepare(&root).await;
        let previous = target.active.db_path().to_path_buf();
        let mut expected_paths = generations(&target.base_dir);
        let failure = Failure::new(&target.base_dir, "before_cleanup");
        target.install_snapshot(&snapshot.meta, snapshot.snapshot).await.unwrap();
        assert_state(&target, true);
        let current = target.get_current_snapshot().await.unwrap().unwrap();
        assert_eq!(snapshot.meta, current.meta);
        let selected = target.active.db_path().to_path_buf();
        expected_paths.insert(selected.clone());
        drop(current);
        drop(target);
        let paths = generations(&root.join("target"));
        assert_eq!(expected_paths, paths);
        assert!(paths.contains(&previous));
        assert!(paths.contains(&selected));
        drop(failure);
        recover(&root, true).await;
        let remaining = generations(&root.join("target"));
        let expected_paths = BTreeSet::from([selected]);
        assert_eq!(expected_paths, remaining);
        fs::remove_dir_all(root).unwrap();
    });
}

#[test]
fn startup_replays_retained_committed_tail_after_state_machine_reopen() {
    TypeConfig::run(async {
        let root = temporary_root();
        let (target, snapshot) = prepare(&root).await;
        drop(snapshot);
        drop(target);
        tracing::info!("Keep the log fixture alive to test committed-tail replay across state-machine reopen");
        let mut logs = log_mem::LogStore::<TypeConfig>::default();
        logs.blocking_append(entries(true)).await.unwrap();
        logs.purge(log_id::<TypeConfig>(1, 1, 2)).await.unwrap();
        logs.save_committed(Some(log_id::<TypeConfig>(1, 1, 5))).await.unwrap();
        let mut state = RocksStateMachine::<TypeConfig>::open(root.join("target")).unwrap();
        assert_state(&state, false);
        let _initial = StorageHelper::new(&mut logs, &mut state).with_id(1).get_initial_state().await.unwrap();
        assert_state(&state, true);
        drop(state);
        recover(&root, true).await;
        fs::remove_dir_all(root).unwrap();
    });
}
