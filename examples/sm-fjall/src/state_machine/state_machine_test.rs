use std::collections::BTreeMap;
use std::io;

use futures::stream;
use openraft::RaftSnapshotBuilder;
use openraft::alias::EntryOf;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;
use tempfile::TempDir;
use types_kv::Request;
use types_kv::VersionedValue;

use super::*;
use crate::TypeConfig;

fn new_state_machine() -> (TempDir, FjallStateMachine<TypeConfig>) {
    let temp_dir = TempDir::new().unwrap();
    let state_machine = FjallStateMachine::<TypeConfig>::open(temp_dir.path()).unwrap();
    (temp_dir, state_machine)
}

/// Apply `(index, request)` pairs as normal entries in one `apply()` call.
async fn apply<const N: usize>(state_machine: &mut FjallStateMachine<TypeConfig>, requests: [(u64, Request); N]) {
    let entries = requests.map(|(index, request)| {
        let entry = EntryOf::<TypeConfig>::new_normal(log_id::<TypeConfig>(1, 1, index), request);
        Ok::<EntryResponder<TypeConfig>, io::Error>((entry, None))
    });
    state_machine.apply(stream::iter(entries)).await.unwrap();
}

async fn build_snapshot(state_machine: &mut FjallStateMachine<TypeConfig>) -> SnapshotOf<TypeConfig, Cursor<Vec<u8>>> {
    state_machine.try_create_snapshot_builder(true).await.unwrap().build_snapshot().await.unwrap()
}

fn versioned(value: &str, version: u64) -> VersionedValue {
    VersionedValue {
        value: value.to_string(),
        version,
    }
}

#[test]
fn cas_sees_earlier_write_in_same_batch() {
    TypeConfig::run(async {
        let (_temp_dir, mut state_machine) = new_state_machine();
        apply(&mut state_machine, [(1, Request::set("key", "A"))]).await;

        // Both entries expect version 1, the index of the `Set`; only the first one succeeds.
        apply(&mut state_machine, [
            (2, Request::compare_and_set("key", 1, "B")),
            (3, Request::compare_and_set("key", 1, "C")),
        ])
        .await;

        assert_eq!(Some(versioned("B", 2)), state_machine.get("key").unwrap());
        assert_eq!(None, state_machine.get("missing").unwrap());

        let (last_applied, _) = state_machine.applied_state().await.unwrap();
        assert_eq!(Some(log_id::<TypeConfig>(1, 1, 3)), last_applied);
    });
}

#[test]
fn builder_captures_state_at_creation() {
    TypeConfig::run(async {
        let (_temp_dir, mut state_machine) = new_state_machine();
        apply(&mut state_machine, [(1, Request::set("key", "old"))]).await;

        let mut builder = state_machine.try_create_snapshot_builder(true).await.unwrap();
        apply(&mut state_machine, [(2, Request::set("key", "new"))]).await;
        let snapshot = builder.build_snapshot().await.unwrap();

        assert_eq!(Some(log_id::<TypeConfig>(1, 1, 1)), snapshot.meta.last_log_id);
        let data: BTreeMap<String, VersionedValue> = deserialize(snapshot.snapshot.get_ref()).unwrap();
        assert_eq!(BTreeMap::from([("key".to_string(), versioned("old", 1))]), data);

        let current = state_machine.get_current_snapshot().await.unwrap().unwrap();
        assert_eq!(snapshot.meta, current.meta);
        assert_eq!(snapshot.snapshot.get_ref(), current.snapshot.get_ref());
    });
}

#[test]
fn install_replaces_all_data() {
    TypeConfig::run(async {
        let (_leader_dir, mut leader) = new_state_machine();
        apply(&mut leader, [(1, Request::set("a", "1")), (2, Request::set("b", "2"))]).await;
        let snapshot = build_snapshot(&mut leader).await;

        let (_follower_dir, mut follower) = new_state_machine();
        apply(&mut follower, [
            (1, Request::set("b", "stale")),
            (2, Request::set("c", "stale")),
        ])
        .await;
        follower.install_snapshot(&snapshot.meta, snapshot.snapshot).await.unwrap();

        assert_eq!(Some(versioned("1", 1)), follower.get("a").unwrap());
        assert_eq!(Some(versioned("2", 2)), follower.get("b").unwrap());
        assert_eq!(None, follower.get("c").unwrap());

        let (last_applied, _) = follower.applied_state().await.unwrap();
        assert_eq!(snapshot.meta.last_log_id, last_applied);
    });
}

#[test]
fn built_snapshot_does_not_replace_newer_installed_one() {
    TypeConfig::run(async {
        let (_leader_dir, mut leader) = new_state_machine();
        apply(&mut leader, [
            (1, Request::set("key", "A")),
            (2, Request::set("key", "B")),
        ])
        .await;
        let installed = build_snapshot(&mut leader).await;

        // A snapshot from the leader is installed while an older one is being built.
        let (_temp_dir, mut state_machine) = new_state_machine();
        apply(&mut state_machine, [(1, Request::set("key", "A"))]).await;
        let mut builder = state_machine.try_create_snapshot_builder(true).await.unwrap();
        state_machine.install_snapshot(&installed.meta, installed.snapshot).await.unwrap();

        let built = builder.build_snapshot().await.unwrap();
        assert_eq!(Some(log_id::<TypeConfig>(1, 1, 1)), built.meta.last_log_id);

        let current = state_machine.get_current_snapshot().await.unwrap().unwrap();
        assert_eq!(installed.meta, current.meta);
    });
}

#[test]
fn reopen_restores_state_and_snapshot() {
    TypeConfig::run(async {
        let temp_dir = TempDir::new().unwrap();

        let snapshot_meta = {
            let mut state_machine = FjallStateMachine::<TypeConfig>::open(temp_dir.path()).unwrap();
            apply(&mut state_machine, [(1, Request::set("key", "A"))]).await;
            let snapshot = build_snapshot(&mut state_machine).await;
            apply(&mut state_machine, [(2, Request::set("key", "B"))]).await;
            snapshot.meta
        };

        let mut state_machine = FjallStateMachine::<TypeConfig>::open(temp_dir.path()).unwrap();
        assert_eq!(Some(versioned("B", 2)), state_machine.get("key").unwrap());

        let (last_applied, _) = state_machine.applied_state().await.unwrap();
        assert_eq!(Some(log_id::<TypeConfig>(1, 1, 2)), last_applied);

        let current = state_machine.get_current_snapshot().await.unwrap().unwrap();
        assert_eq!(snapshot_meta, current.meta);
    });
}
