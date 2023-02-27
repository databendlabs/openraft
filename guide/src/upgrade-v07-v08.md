# Guide for upgrading from [v0.7.*](https://github.com/datafuselabs/openraft/tree/v0.7.4) to [v0.8.*](https://github.com/datafuselabs/openraft/tree/v0.8.0):

[Change log v0.8.0](https://github.com/datafuselabs/openraft/blob/release-0.8/change-log.md#v080)

In this chapter, for users who will upgrade openraft 0.7 to openraft 0.8,
we are going to explain what changes has been made from openraft-0.7 to
openraft-0.8 and why these changes are made.

- **Backup your data before deploying upgraded application**.

To upgrade:

1. Update the application to adopt `v0.8.*` openraft.
  The updated `RaftStorage` implementation must pass [`RaftStorage` test suite](https://github.com/datafuselabs/openraft/blob/release-0.8/memstore/src/test.rs),
  and the compatibility test: [compatibility test](https://github.com/datafuselabs/openraft/blob/main/rocksstore-compat07/src/compatibility_test.rs)

2. Then shutdown all `v0.7.*` nodes and then bring up `v0.8.*` nodes.

  `v0.7.*` and `v0.8.*` should **NEVER** run in a same cluster, due to the data structure changes.
  Exchanging data between `v0.7.*` and `v0.8.*` nodes may lead to data damage.


## Upgrade steps

In general, the upgrade includes the following steps:

- Keep in mind that v0.8 is compatible with v0.7 only when the application using `serde` to serialize data.

- Enable feature flag `compat-07`.

- Optionally enable feature flag `single-term-leader` if the application wants to use standard raft. See [Multi/single leader in each term](#multisingle-leader-in-each-term) chapter.

- Add type config to define what types to use for openraft, See [RaftTypeConfig](https://github.com/datafuselabs/openraft/blob/47d6c9f32d9675462ab5d64a1f6a4be7574f1ab2/openraft/src/raft.rs#L81) :

  ```rust
  openraft::declare_raft_types!(
      pub MyTypeConfig: D = ClientRequest, R = ClientResponse, NodeId = u64, Node = openraft::EmptyNode
  );
  ```

- Add generics parameter to types such as:
  `LogId -> LogId<NID>`, `Membership -> Membership<NID, N>`
  `Entry<D> -> Entry<MyTypeConfig>`

- Move `RaftStorage` methods implementation according to the
    [Storage API changes](#storage-api-changes) chapter.

  - Replace `HardState` with `Vote`, and `[read/save]_hard_state` with `[read/write]_vote`.
  - Replace `EffectiveMembership` with `StoredMembership`.

- Move `RaftNetwork` methods implementation according to the
    [Network-API-changes](#network-api-changes) chapter.

- Replace types with the ones provided by [`openraft::compat::compat07`](https://github.com/datafuselabs/openraft/blob/release-0.8/openraft/src/compat/compat07.rs).

- Finally, make sure the `RaftStorage` implementation passes
[`RaftStorage` test suite](https://github.com/datafuselabs/openraft/blob/release-0.8/memstore/src/test.rs) and
[compatibility test](https://github.com/datafuselabs/openraft/blob/main/rocksstore-compat07/src/compatibility_test.rs)


# Compatibility with v0.7 format data

Openraft v0.8 can be compatible with v0.7 if:
- The application uses `serde` to serialize data types
- Enabling `compat-07` feature flags.

Openraft uses a `RaftStorage` implementation provided by the application to
store presistent data. When upgrading from v0.7 to v0.8, it is important to
ensure that the updated `RaftStorage` is backward compatible and can read the
data written by v0.7 openraft, in addition to reading and writing v0.8 openraft data.
This ensures that the application continues to function smoothly after upgrade.

## Openraft v0.8 compatible mode

Compared to v0.7, openraft v0.8 has a more generic design.
However, it is still possible to build it in a v0.7 compatible mode by enabling
the feature flag `compat-07`.
More information can be found at: [feature-flag-compat-07](https://datafuselabs.github.io/openraft/feature-flags).

It is worth noting that an application does **NOT** need to enable this feature flag
if it chooses to manually upgrade the v0.7 format data.

**Generic design in v0.8** includes:
- generic type `NodeId` and `Node` were introduced,
- `serde` became an optional.

Because of these generalization, feature `compat-07` enables the following feature flags:

- `serde`: it adds `serde` implementation to types such as `LogId`.

And v0.8 will be compatible with v0.7 only when it uses `u64` as `NodeId` and `openraft::EmptyNode` as `Node`.


## Implement a compatible storage layer

In addition to enabling `compat-07` feature flag, openraft provides a compatible layer in
[`openraft::compat::compat07`](https://github.com/datafuselabs/openraft/blob/release-0.8/openraft/src/compat/compat07.rs) to help application developer to upgrade.
This mod provides several types that can deserialize from both v0.7 format data and the latest format data.

An application uses these types to replace the corresponding ones in a
`RaftStorage` implementation, so that v0.7 data and v0.8 data can both be read.

For example, in a compatible storage implementation, reading a `LogId` should be
done in the following way:

```rust
use openraft::compat::compat07;

fn read_log_id(bs: &[u8]) -> openraft::LogId<u64> {
    let log_id: compat07::LogId = serde_json::from_slice(&bs).expect("incompatible");
    let latest: openraft::LogId<u64> = log_id.upgrade();
    latest
}
```

## Example of compatible storage

[rocksstore-compat07](https://github.com/datafuselabs/openraft/tree/main/rocksstore-compat07)
is a good example using these compatible type to implement a compatible `RaftStorage`.

This is an example `RaftStorage` implementation that can read persistent
data written by either v0.7.4 or v0.8 the latest version openraft.

rocksstore-compat07 is built with openraft 0.8, in the tests, it reads
data written by rocksstore 0.7.4, which is built with openraft 0.7.4 .

In this example, it loads data through the compatibility layer:
`openraft::compat::compat07`, which defines several compatible types
that can be deserialized from v0.7 or v0.8 data, such as
`compat07::LogId` or `compat07::Membership`.


## Test compatibility

Openraft also provides a testing suite [`testing::Suite07`](https://github.com/datafuselabs/openraft/blob/47d6c9f32d9675462ab5d64a1f6a4be7574f1ab2/openraft/src/compat/compat07.rs#L291) to ensure old data will be correctly read.
An application should ensure that its storage passes this test suite.
Just like [rocksstore-compat07/compatibility_test.rs](https://github.com/datafuselabs/openraft/blob/main/rocksstore-compat07/src/compatibility_test.rs) does.

To test compatibility of the application storage API:

- Define a builder that builds a v0.7 `RaftStorage` implementation.
- Define another builder that builds a v0.8 `RaftStorage` implementation.
- Run tests in `compat::testing::Suite07` with these two builder. In this test
    suite, it writes data with a v0.7 storage API and then reads them with an
    v0.8 storage API.

```rust
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

## Generic Node

Openraft v0.8 introduces generic type `openraft::NodeId` and `openraft::Node`, an application now
can used any type for a node-id or node object.

A type that needs `NodeId` or `Node` now has generic type parameter in them, 
e.g, `struct Membership {...}` became:

```rust
pub struct Membership<NID, N>
where N: Node, NID: NodeId
{...}
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
it reduces LogId size from `LogId:{term, node_id, index}` to `LogId{term, index}`.
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

  ```rust
  openraft::declare_raft_types!(
      pub MyTypeConfig: D = ClientRequest, R = ClientResponse, NodeId = u64, Node = openraft::EmptyNode
  );
  ```
