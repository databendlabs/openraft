use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;

use log_fjall::FjallLogStore;
use openraft::StorageError;
use openraft::alias::EntryOf;
use openraft::entry::RaftEntry;
use openraft::impls::Membership;
use openraft::testing::log::ApplicationStoreBuilder;
use openraft::testing::log::ApplicationSuite;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;
use tempfile::TempDir;
use types_kv::Request;
use types_kv::Response;
use types_kv::VersionedValue;

use super::FjallStateMachine;
use super::deserialize;
use crate::TypeConfig;

struct FjallApplicationBuilder;

impl StoreBuilder<TypeConfig, FjallLogStore<TypeConfig>, FjallStateMachine<TypeConfig>, TempDir>
    for FjallApplicationBuilder
{
    async fn build(
        &self,
    ) -> Result<(TempDir, FjallLogStore<TypeConfig>, FjallStateMachine<TypeConfig>), StorageError<TypeConfig>> {
        let temporary = TempDir::new().map_err(storage_error)?;
        let logs = FjallLogStore::open(temporary.path().join("log")).map_err(storage_error)?;
        let state = FjallStateMachine::open(temporary.path().join("sm")).map_err(storage_error)?;
        Ok((temporary, logs, state))
    }
}

impl ApplicationStoreBuilder<TypeConfig, FjallLogStore<TypeConfig>, FjallStateMachine<TypeConfig>, TempDir>
    for FjallApplicationBuilder
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

    async fn read_state(&self, state: &mut FjallStateMachine<TypeConfig>) -> io::Result<Self::State> {
        let mut result = BTreeMap::new();
        for item in state.data.iter() {
            let (key, value) = item.into_inner().map_err(io::Error::other)?;
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
        ApplicationSuite::test_all(&FjallApplicationBuilder).await.unwrap();
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
