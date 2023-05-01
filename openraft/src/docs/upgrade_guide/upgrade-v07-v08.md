# Guide for upgrading from [v0.7](https://github.com/datafuselabs/openraft/tree/v0.7.4) to [v0.8](https://github.com/datafuselabs/openraft/tree/release-0.8):

[Change log v0.8](https://github.com/datafuselabs/openraft/blob/release-0.8/change-log.md)

In this chapter, for users who will upgrade openraft 0.7 to openraft 0.8,
we are going to explain what changes has been made from openraft-0.7 to
openraft-0.8 and why these changes are made.

- **Backup your data before deploying upgraded application**.

To upgrade:

1. Update the application to adopt `v0.8` openraft.
   The updated `RaftStorage` implementation must pass [`testing::Suite`](`crate::testing::Suite`):
   an example of running it is [`RaftStorage` test suite](https://github.com/datafuselabs/openraft/blob/release-0.8/memstore/src/test.rs),
   and the compatibility test: [compatibility test](https://github.com/datafuselabs/openraft/blob/main/rocksstore-compat07/src/compatibility_test.rs)

2. Then shutdown all `v0.7` nodes and then bring up `v0.8` nodes.

`v0.7` and `v0.8` should **NEVER** run in a same cluster, due to the data structure changes.
Exchanging data between `v0.7` and `v0.8` nodes may lead to data damage.


## Upgrade steps

In general, the upgrade includes the following steps:

### Prepare v0.8

Openraft `v0.8` provides several [`feature_flags`] to provide compatibility with `v0.7`.

- `serde`: Make sure that the application uses `serde` to serialize data; Openraft v0.8 provides a compatibility layer that is built upon `serde`.

- `compat-07`: Enable feature flag `compat-07` to enable the compatibility layer [`compat::compat07`](`crate::compat::compat07`).

- Optionally enable feature flag `single-term-leader` if the application wants to use standard raft. See [Multi/single leader in each term](#multisingle-leader-in-each-term) chapter.

### Upgrade the application codes

- Add type config to define concrete types to use for openraft, See [`RaftTypeConfig`].

  Openraft `v0.8` introduces several more generic types to define application types that are defined as associated types of `RaftTypeConfig`.
  An `v0.8` application should implement [`RaftTypeConfig`]:

  ```ignore
  pub(crate) struct MyTypeConfig {}
  impl RaftTypeConfig for MyTypeConfig {
      type D = ClientRequest;
      type R = ClientResponse;
      type NodeId = u64;
      type Node = openraft::EmptyNode;
      type Entry = openraft::entry::Entry<MyTypeConfig>;
      type SnapshotData = Cursor<Vec<u8>>;
  }
  ```

  To upgrade to `v0.8` compatible with `v0.7`, the application must use the same
  or compatible type for these new generic types, such as

  - use `u64` for [`RaftTypeConfig::NodeId`],
  - use [`EmptyNode`] for `RaftTypeConfig::Node`,
  - use [`entry::Entry<MyTypeConfig>`] for [`RaftTypeConfig::Entry`], where `MyTypeConfig` is the type config defined above,
  - use `RaftStorage::SnapshotData` that is used in `v0.7` for [`RaftTypeConfig::SnapshotData`].

  | Data type     | `v0.7`                      | `v0.8`                                                              |
  | ---           | ---                         | ---                                                                 |
  | Node id       | `u64`                       | [`RaftTypeConfig::NodeId`]       = `u64`                            |
  | Node          | no such type                | [`RaftTypeConfig::Node`]         = [`EmptyNode`]                    |
  | Log entry     | `openraft::Entry`           | [`RaftTypeConfig::Entry`]        = [`entry::Entry<MyTypeConfig>`]   |
  | Snapshot data | `RaftStorage::SnapshotData` | [`RaftTypeConfig::SnapshotData`] = `v07::RaftStorage::SnapshotData` |


  Openraft `v0.8` also provides a macro [`declare_raft_types`](`crate::declare_raft_types`) to declare these types:

  ```ignore
  openraft::declare_raft_types!(
      pub MyTypeConfig:
          D = ClientRequest,
          R = ClientResponse,
          NodeId = u64,
          Node = openraft::EmptyNode,
          Entry = openraft::entry::Entry<MyTypeConfig>,
          SnapshotData = Cursor<Vec<u8>>,
  );
  ```

- To upgrade to `v0.8`, data types have generic type parameters need to be updated, such as:
    - `LogId` becomes `LogId<u64>`,
    - `Membership` becomes `Membership<u64, openraft::EmptyNode>`,
    - `Entry<D>` becomes `Entry<MyTypeConfig>`.
  Where `u64` is node id type in `v0.7` and `MyTypeConfig` is the type config defined in the previous step.

- Update `RaftStorage` methods implementation according to the
  [Storage API changes](#storage-api-changes) chapter.

    - Replace `HardState` with `Vote`, and `[read/save]_hard_state` with `[read/write]_vote`.
    - Replace `EffectiveMembership` with `StoredMembership`.

  In order to ensure compatibility with version `0.7`, the storage implementation must replace the types used for deserialization with those supplied by [`compat::compat07`].
  [`compat::compat07`] includes types like [`compat::compat07::Entry`] that can be deserialized from both `v0.7` serialized `Entry` and `v0.8` serialized `Entry`.

- Move `RaftNetwork` methods implementation according to the
  [Network-API-changes](#network-api-changes) chapter.

- Use `v0.8` [`Adaptor`] to install `RaftStorage` to `Raft`.

  See: in `v0.8` [`Raft`] splits `RaftStorage` into two parts: the log store and
  the state machine store:

  `v0.8` `Raft`:
  ```ignore
  pub struct Raft<C, N, LS, SM>
  where
      C: RaftTypeConfig,
      N: RaftNetworkFactory<C>,
      LS: RaftLogStorage<C>,
      SM: RaftStateMachine<C>;
  ```

  `v0.7` `Raft`:
  ```ignore
  pub struct Raft<C, N, S>
  where
      C: RaftTypeConfig,
      N: RaftNetworkFactory<C>,
      S: RaftStorage<C>,
  ```

  Use [`Adaptor`] to create [`RaftLogStorage`] and [`RaftStateMachine`] to create a [`Raft`] instance:

  ```ignore
  use openraft::{Config, Raft};
  use openraft::storage::Adaptor;

  let store = MyRaftStorage::new();
  let (log_store, state_machine) = Adaptor::new(store);
  Raft::new(1, Arc::new(Config::default()), MyNetwork::default(), log_store, state_machine);
  ```

- Finally, make sure the `RaftStorage` implementation pass [`RaftStorage` test suite] and [compatibility test]


# Compatibility with v0.7 format data

Openraft `v0.8` can be built compatible with `v0.7` if:
- The application uses `serde` to serialize data types
- Enabling `compat-07` feature flags.

To ensure the compatibility after upgrading from `v0.7` to `v0.8` in Openraft, it's crucial to confirm that the updated `RaftStorage` implementation is backward compatible.
This means that it can both read the data written by `v0.7` Openraft and read and write `v0.8` Openraft data.
The `RaftStorage` implementation is responsible for storing persistent data in Openraft.

## Openraft v0.8 compatible mode

Compared to `v0.7`, openraft `v0.8` has a more generic design.
However, it is still possible to build it in a `v0.7` compatible mode by enabling
the feature flag `compat-07`.
More information can be found at: [feature-flag-compat-07][`feature_flags`].

It is worth noting that an application does **NOT** need to enable this feature flag
if it chooses to manually upgrade the `v0.7` format data.

**Generic design in v0.8** includes:

- generic type `NodeId`, `Node` and `Entry` were introduced,
- `serde` became an optional.

Because of these generalizations, feature `compat-07` enables the following feature flags:

- `serde`: it adds `serde` implementation to types such as `LogId`.

And `v0.8` will be compatible with `v0.7` only when it uses `u64` as `NodeId` and `openraft::EmptyNode` as `Node`.


## Implement a compatible storage layer

In addition to enabling `compat-07` feature flag, openraft provides a compatible layer in
[`compat::compat07`] to help application developer to upgrade.
This mod provides several types that can deserialize from both `v0.7` format data and the latest format data.

An application uses these types to replace the corresponding ones in a
`RaftStorage` implementation, so that `v0.7` data and `v0.8` data can both be read.

For example, in a compatible storage implementation, reading a `LogId` should be
done with 2 steps:

- Loading and deserialize with `compat07::LogId`,
- upgrade `compat07::LogId` to the `v0.8` `LogId`:

```ignore
use openraft::compat::compat07;

fn read_log_id(bs: &[u8]) -> openraft::LogId<u64> {
    let log_id: compat07::LogId = serde_json::from_slice(&bs).expect("incompatible");
    let latest: openraft::LogId<u64> = log_id.upgrade();
    latest
}
```

The following types should be updated in a `RaftStorage` implementation:

```ignore
compat07::Entry;
compat07::EntryPayload;
compat07::LogId;
compat07::Membership;
compat07::SnapshotMeta;
compat07::StoredMembership;
compat07::Vote;
```

## Example of compatible storage

[rocksstore-compat07](https://github.com/datafuselabs/openraft/tree/release-0.8/rocksstore-compat07)
is a good example using these compatible type to implement a compatible `RaftStorage`.

This is an example `RaftStorage` implementation that can read persistent
data written by either `v0.7` or `v0.8` the latest version openraft.

rocksstore-compat07 is built with openraft 0.8, in the tests, it reads
data written by rocksstore 0.7.4, which is built with openraft 0.7.4 .

In this example, it loads data through the compatibility layer:
[`compat::compat07`], which defines several compatible types
that can be deserialized from `v0.7` or `v0.8` data, such as
`compat07::LogId` or `compat07::Membership`.


## Test compatibility

Openraft also provides a testing suite
[`compat::testing::Suite07`] to ensure old data will be correctly read.
An application should ensure that its storage passes this test suite.  Just like [rocksdb-compatability-test][compatibility test] does.

To test compatibility of the application storage API:

- Define a builder that builds a `v0.7` `RaftStorage` implementation.
- Define another builder that builds a v0.8 `RaftStorage` implementation.
- Run tests in [`compat::testing::Suite07`] with these two builder. In this test
  suite, it writes data with a `v0.7` storage API and then reads them with an
  `v0.8` storage API.

```ignore
use openraft::compat;

struct Builder07;
struct BuilderLatest;

#[async_trait::async_trait]
impl compat::testing::StoreBuilder07 for Builder07 {
    type D = rocksstore07::RocksRequest;
    type R = rocksstore07::RocksResponse;
    type S = Arc<rocksstore07::RocksStore>;

    async fn build(&self, p: &Path) -> Arc<rocksstore07::RocksStore> {
        rocksstore07::RocksStore::new(p).await
    }

    fn sample_app_data(&self) -> Self::D {
        rocksstore07::RocksRequest::Set { key: s("foo"), value: s("bar") }
    }
}

#[async_trait::async_trait]
impl compat::testing::StoreBuilder for BuilderLatest {
    type C = crate::Config;
    type S = Arc<crate::RocksStore>;

    async fn build(&self, p: &Path) -> Arc<crate::RocksStore> {
        crate::RocksStore::new(p).await
    }

    fn sample_app_data(&self) -> <<Self as compat::testing::StoreBuilder>::C as openraft::RaftTypeConfig>::D {
        crate::RocksRequest::Set { key: s("foo"), value: s("bar") }
    }
}

#[tokio::test]
async fn test_compatibility_with_rocksstore_07() -> anyhow::Result<()> {
    compat::testing::Suite07 {
        builder07: Builder07,
        builder_latest: BuilderLatest,
    }.test_all().await?;
    Ok(())
}

fn s(v: impl ToString) -> String {
    v.to_string()
}
```

# Summary of changes introduced in v0.8

## Generics

Openraft v0.8 introduces trait `openraft::NodeId`, `openraft::Node`, `openraft::entry::RaftEntry`, an application now
can use any type for a node-id, node object and log entry.

A type that needs `NodeId` or `Node` now has generic type parameter in them,
e.g, `struct Membership {...}` became:

```ignore
pub struct Membership<NID, N>
where N: Node, NID: NodeId{}
```

## Optional `serde`

`serde` in openraft v0.8 became optional. To enable `serde`, build openraft with
`serde` feature flag. See: [feature flags](https://datafuselabs.github.io/openraft/feature-flags).


## Multi/single leader in each term

Openraft v0.8 by default allows multiple leaders to be elected in a single `term`, in order to minimize conflicting during election.
To run in the standard raft mode, i.e, only one leader can be elected in every
term, an application builds openraft with feature flag "single-term-leader".
See: [feature flags](https://datafuselabs.github.io/openraft/feature-flags).

With this feature on: only one leader can be elected in each term, but
it reduces LogId size from `LogId{term, node_id, index}` to `LogId{term, index}`.
It will be preferred if an application uses a big `NodeId` type.

Openraft v0.7 runs in the standard raft mode, i.e., at most one leader per term.
It is safe to upgrade v0.7 single-term-leader mode to v0.8 multi-term-leader
mode.

## Storage API changes

### Split `RaftStorage`

Extract `RaftLogReader`, `RaftSnapshotBuilder` from `RaftStorage`

`RaftStorage` is now refactored to:
- `RaftLogReader` to read data from the log in parallel tasks independent of the main Raft loop
- `RaftStorage` to modify the log and the state machine (implements also `RaftLogReader`) intended to be used in the main Raft loop
- `RaftSnapshotBuilder` to build the snapshot in background independent of the main Raft loop

The `RaftStorage` API offers to create new `RaftLogReader` or `RaftSnapshotBuilder` on it.

### Move default implemented methods from `RaftStorage` to `StorageHelper`

Function `get_log_entries()` and `try_get_log_entry()` are provided with default implementations. However, they do not need to be part of this trait and an application does not have to implement them.


## Network API changes

### Split `RaftNetwork` and `RaftNetworkFactory`

`RaftNetwork` is also refactored to:
- `RaftNetwork` responsible for sending RPCs
- `RaftNetworkFactory` responsible for creating instances of `RaftNetwork` for sending data to a particular node.


## Data type changes

- When building a `SnapshotMeta`, another field is required: `last_membership`, for storing the last applied membership.

- `HardState` is replaced with `Vote`.

- Add `SnapshotSignature` to identify a snapshot for transport.

- Replace usages of `EffectiveMembership` in `RaftStorage` with `StoredMembership`.

- Introduce generalized types `NodeId` and `Node` to let user defines arbitrary
  node-id or node. Openraft v0.8 relies on a `RaftTypeConfig` for your application to define types that are used by openraft, with the following macro:

  ```ignore
  openraft::declare_raft_types!(
      pub MyTypeConfig:
          D = ClientRequest,
          R = ClientResponse,
          NodeId = u64,
          Node = openraft::EmptyNode,
          Entry = openraft::entry::Entry<MyTypeConfig>,
          SnapshotData = Cursor<Vec<u8>>
  );
  ```

[`feature_flags`]: `crate::docs::feature_flags`

[`RaftTypeConfig`]:               `crate::raft::RaftTypeConfig`
[`RaftTypeConfig::NodeId`]:       `crate::raft::RaftTypeConfig::NodeId`
[`RaftTypeConfig::Node`]:         `crate::raft::RaftTypeConfig::Node`
[`RaftTypeConfig::Entry`]:        `crate::raft::RaftTypeConfig::Entry`
[`RaftTypeConfig::SnapshotData`]: `crate::raft::RaftTypeConfig::SnapshotData`

[`EmptyNode`]:                  `crate::EmptyNode`
[`entry::Entry`]:               `crate::entry::Entry`
[`entry::Entry<MyTypeConfig>`]: `crate::entry::Entry`

[`Adaptor`]: `crate::storage::Adaptor`

[`Raft`]: `crate::Raft`
[`RaftLogStorage`]: `crate::storage::RaftLogStorage`
[`RaftStateMachine`]: `crate::storage::RaftStateMachine`

[`declare_raft_types`]: `crate::declare_raft_types`
[`compat::compat07`]: `crate::compat::compat07`
[`compat::compat07::Entry`]: `crate::compat::compat07::Entry`

[compatibility test]: https://github.com/datafuselabs/openraft/blob/release-0.8/rocksstore-compat07/src/compatibility_test.rs
[`RaftStorage` test suite]: https://github.com/datafuselabs/openraft/blob/release-0.8/memstore/src/test.rs


[`compat::testing::Suite07`]: `crate::compat::testing::Suite07`
