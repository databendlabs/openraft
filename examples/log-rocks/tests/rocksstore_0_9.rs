//! Reopen a log store that `openraft-rocksstore` 0.9.21 wrote.
//!
//! `fixtures/rocksstore-0.9.21` is the RocksDB directory that
//! `fixtures/gen-rocksstore-0.9.21` writes with `openraft-rocksstore` 0.9.21: a committed vote,
//! a blank, a membership and two normal entries, the first entry purged, and a state machine
//! that applied four entries and built a snapshot. `log-rocks` keeps the `meta` and `logs`
//! column families of that store, so it reads the vote and the logs as 0.9 left them.
//!
//! The state machine is not reopened: `openraft-rocksstore` keeps it in the `sm_meta` and
//! `sm_data` column families of the same database, while `sm-rocks` keeps one database per
//! generation in its own directory.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

use log_rocks::RocksLogStore;
use openraft::BasicNode;
use openraft::EntryPayload;
use openraft::Membership;
use openraft::RaftLogReader;
use openraft::RaftTypeConfig;
use openraft::Vote;
use openraft::alias::LogIdOf;
use openraft::storage::RaftLogStorage;
use openraft::type_config::TypeConfigExt;
use openraft::vote::RaftLeaderIdExt;
use rocksdb::DB;
use rocksdb::Options;
use serde_json::json;

openraft::declare_raft_types!(
    /// `openraft-rocksstore` 0.9.21 uses `u64` node ids and `BasicNode`. Its request,
    /// `RocksRequest::Set { key, value }`, is read here as plain JSON.
    pub TypeConfig:
        D = serde_json::Value,
        R = serde_json::Value,
);

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/rocksstore-0.9.21");

fn log_id(term: u64, index: u64) -> LogIdOf<TypeConfig> {
    LogIdOf::<TypeConfig>::new(<TypeConfig as RaftTypeConfig>::LeaderId::new_committed(term, 1), index)
}

/// Copy the fixture, because opening a RocksDB directory writes to it.
fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        fs::copy(entry.path(), to.join(entry.file_name()))?;
    }
    Ok(())
}

#[test]
fn reopen_rocksstore_0_9_21_logs() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    copy_dir(Path::new(FIXTURE), dir.path())?;

    // RocksDB opens a database only with all of its column families, the state machine's too.
    let column_families = ["meta", "sm_meta", "sm_data", "logs"];
    let db = DB::open_cf(&Options::default(), dir.path(), column_families).map_err(io::Error::other)?;
    let mut store = RocksLogStore::<TypeConfig>::new(Arc::new(db));

    TypeConfig::run(async move {
        let vote = store.read_vote().await?;
        assert_eq!(Some(Vote::new_committed(2, 1)), vote);

        let state = store.get_log_state().await?;
        assert_eq!(Some(log_id(1, 1)), state.last_purged_log_id);
        assert_eq!(Some(log_id(2, 5)), state.last_log_id);

        let entries = store.try_get_log_entries(..).await?;
        let log_ids = entries.iter().map(|e| e.log_id).collect::<Vec<_>>();
        assert_eq!(vec![log_id(1, 2), log_id(2, 3), log_id(2, 4), log_id(2, 5)], log_ids);

        let nodes = (1..=3).map(|id| (id, BasicNode::new(format!("127.0.0.1:{}", 21000 + id))));
        let membership = Membership::new(vec![BTreeSet::from([1, 2, 3])], BTreeMap::from_iter(nodes)).unwrap();
        let payloads = entries.into_iter().map(|e| e.payload).collect::<Vec<_>>();
        assert_eq!(
            vec![
                EntryPayload::Membership(membership),
                EntryPayload::Blank,
                EntryPayload::Normal(json!({"Set": {"key": "foo", "value": "bar"}})),
                EntryPayload::Normal(json!({"Set": {"key": "baz", "value": "qux"}})),
            ],
            payloads
        );

        Ok(())
    })
}
