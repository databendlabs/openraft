use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;

use log_rocks::RocksLogStore;
use openraft::StorageError;
use openraft::alias::EntryOf;
use openraft::entry::RaftEntry;
use openraft::impls::Membership;
use openraft::testing::log::ApplicationStoreBuilder;
use openraft::testing::log::ApplicationSuite;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;
use rocksdb::IteratorMode;
use tempfile::TempDir;
use types_kv::Request;
use types_kv::Response;
use types_kv::VersionedValue;

use super::RocksStateMachine;
use super::column_family;
use super::deserialize;
use crate::TypeConfig;

struct RocksApplicationBuilder;

impl StoreBuilder<TypeConfig, RocksLogStore<TypeConfig>, RocksStateMachine<TypeConfig>, TempDir>
    for RocksApplicationBuilder
{
    async fn build(
        &self,
    ) -> Result<(TempDir, RocksLogStore<TypeConfig>, RocksStateMachine<TypeConfig>), StorageError<TypeConfig>> {
        let temporary = TempDir::new().map_err(storage_error)?;
        let logs = RocksLogStore::open(temporary.path().join("log")).map_err(storage_error)?;
        let state = RocksStateMachine::open(temporary.path().join("sm")).map_err(storage_error)?;
        Ok((temporary, logs, state))
    }
}

impl ApplicationStoreBuilder<TypeConfig, RocksLogStore<TypeConfig>, RocksStateMachine<TypeConfig>, TempDir>
    for RocksApplicationBuilder
{
    type State = BTreeMap<String, VersionedValue>;

    fn entries(&self) -> Vec<EntryOf<TypeConfig>> {
        entries()
    }

    fn expected_responses(&self) -> Vec<Response> {
        expected_responses()
    }

    fn expected_state(&self) -> Self::State {
        expected_state()
    }

    async fn read_state(&self, state: &mut RocksStateMachine<TypeConfig>) -> io::Result<Self::State> {
        let db = state.active.db();
        let data = column_family(db, "sm_data")?;
        let mut result = BTreeMap::new();
        for item in db.iterator_cf(data, IteratorMode::Start) {
            let (key, value) = item.map_err(io::Error::other)?;
            let key = String::from_utf8(key.to_vec()).map_err(io::Error::other)?;
            let value = deserialize(&value)?;
            result.insert(key, value);
        }
        Ok(result)
    }
}

#[test]
fn application_responses_do_not_depend_on_apply_grouping() {
    TypeConfig::run(async {
        ApplicationSuite::test_all(&RocksApplicationBuilder).await.unwrap();
    });
}

fn entries() -> Vec<EntryOf<TypeConfig>> {
    let members = Membership::new_with_defaults(vec![BTreeSet::from([1])], []);
    vec![
        EntryOf::<TypeConfig>::new_membership(log_id::<TypeConfig>(1, 1, 1), members),
        entry(2, Request::set("key", "A")),
        entry(3, Request::compare_and_set("key", 2, "B")),
        entry(4, Request::compare_and_set("key", 2, "C")),
        entry(5, Request::set("first", "one")),
        entry(6, Request::set("second", "two")),
        entry(7, Request::set("third", "three")),
    ]
}

fn entry(index: u64, request: Request) -> EntryOf<TypeConfig> {
    let log_id = log_id::<TypeConfig>(1, 1, index);
    EntryOf::<TypeConfig>::new_normal(log_id, request)
}

fn expected_responses() -> Vec<Response> {
    vec![
        Response::none(),
        Response::new("A", 2),
        Response::new("B", 3),
        Response::none(),
        Response::new("one", 5),
        Response::new("two", 6),
        Response::new("three", 7),
    ]
}

fn expected_state() -> BTreeMap<String, VersionedValue> {
    BTreeMap::from([
        ("first".to_string(), versioned("one", 5)),
        ("key".to_string(), versioned("B", 3)),
        ("second".to_string(), versioned("two", 6)),
        ("third".to_string(), versioned("three", 7)),
    ])
}

fn versioned(value: &str, version: u64) -> VersionedValue {
    VersionedValue {
        value: value.to_string(),
        version,
    }
}

fn storage_error(error: io::Error) -> StorageError<TypeConfig> {
    let source = TypeConfig::err_from_error(&error);
    StorageError::read(source)
}
