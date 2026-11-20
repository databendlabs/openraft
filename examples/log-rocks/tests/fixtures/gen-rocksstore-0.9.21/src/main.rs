//! Write a RocksDB directory with `openraft-rocksstore` 0.9.21, as a 0.9
//! application would have left it.
//!
//! The log holds a blank entry, a membership entry and normal entries from two
//! terms. The state machine has applied a prefix of them and built a snapshot,
//! and the log store purged the first entry.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;

use openraft::BasicNode;
use openraft::CommittedLeaderId;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftSnapshotBuilder;
use openraft::RaftStorage;
use openraft::Vote;
use openraft_rocksstore::RocksRequest;
use openraft_rocksstore::RocksStore;
use openraft_rocksstore::TypeConfig;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::new(CommittedLeaderId::new(term, 1), index)
}

fn set(key: &str, value: &str) -> EntryPayload<TypeConfig> {
    EntryPayload::Normal(RocksRequest::Set {
        key: key.to_string(),
        value: value.to_string(),
    })
}

#[tokio::main]
async fn main() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rocksstore-0.9.21");
    if out.exists() {
        std::fs::remove_dir_all(&out).unwrap();
    }

    let nodes = (1..=3).map(|id| (id, BasicNode::new(format!("127.0.0.1:{}", 21000 + id))));
    let membership = Membership::new(vec![BTreeSet::from([1, 2, 3])], BTreeMap::from_iter(nodes));

    let entries = vec![
        Entry {
            log_id: log_id(1, 1),
            payload: EntryPayload::Blank,
        },
        Entry {
            log_id: log_id(1, 2),
            payload: EntryPayload::Membership(membership),
        },
        Entry {
            log_id: log_id(2, 3),
            payload: EntryPayload::Blank,
        },
        Entry {
            log_id: log_id(2, 4),
            payload: set("foo", "bar"),
        },
        Entry {
            log_id: log_id(2, 5),
            payload: set("baz", "qux"),
        },
    ];

    {
        let mut store = RocksStore::new(&out).await;

        store.save_vote(&Vote::new_committed(2, 1)).await.unwrap();
        store.append_to_log(entries.clone()).await.unwrap();
        store.apply_to_state_machine(&entries[..4]).await.unwrap();
        store.get_snapshot_builder().await.build_snapshot().await.unwrap();
        store.purge_logs_upto(log_id(1, 1)).await.unwrap();
    }

    // RocksDB's info logs differ on every run and are not needed to reopen it.
    for entry in std::fs::read_dir(&out).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        if name == "LOCK" || name.starts_with("LOG") {
            std::fs::remove_file(&path).unwrap();
        }
    }
}
