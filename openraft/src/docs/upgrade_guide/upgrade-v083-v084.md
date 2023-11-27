# Guide for upgrading from [v0.8.3](https://github.com/datafuselabs/openraft/tree/v0.8.3) to [v0.8.4](https://github.com/datafuselabs/openraft/tree/v0.8.4):

[Change log v0.8.4](https://github.com/datafuselabs/openraft/blob/release-0.8/change-log.md)

You can find the summary and details of changes in the above link.
In this guide, we will focus on the breaking changes that require your attention.

1. **Change [a92499f2](https://github.com/datafuselabs/openraft/commit/a92499f20360f0f6eba3452e6944702d6a50f56d): [`StoreBuilder`][] Modification**
    - **Description**: The `StoreBuilder` builds a `RaftStorage` for test. It no longer needs to run a test; it only needs to build a store.
    - **Upgrade Guide**: Simplify your store building by removing the test.
    - **Example Code**: [memstore/src/test.rs](https://github.com/datafuselabs/openraft/commit/a92499f20360f0f6eba3452e6944702d6a50f56d?diff=split#diff-302d9b55705a5d613394aecc4d346dc6e742cbc767cc292bccae5bb535b761a1)
      ```diff
       impl StoreBuilder<Config, Arc<YourStore>> for MemBuilder {
      -    async fn run_test<Fun, Ret, Res>(&self, t: Fun) -> Result<Ret, StorageError<MemNodeId>>
      -    where
      -        Res: Future<Output = Result<Ret, StorageError<MemNodeId>>> + Send,
      -        Fun: Fn(Arc<YourStore>) -> Res + Sync + Send,
      -    {
      +    async fn build(&self) -> Result<((), Arc<YourStore>), StorageError<MemNodeId>> {
               let store = YourStore::new_async().await;
      -        t(store).await
      +        Ok(((), store))
        }
        }
      ```

1. **Change [6e9d3573](https://github.com/datafuselabs/openraft/commit/6e9d35736a0967f06d9e334a0286208e7d6fe123): Removal of `Clone` from `AppData`**
    - **Description**: The `Clone` trait has been removed from the `AppData` trait.
    - **Upgrade Guide**: Your implementations of `AppData` does not have to derive `Clone` anymore.
    - **Example Code**:
      ```diff
      - #[derive(Clone)]
      struct MyAppData;
      ```

1. **Change [285e6225](https://github.com/datafuselabs/openraft/commit/285e6225d3e0e8363d8b194e09a113f4e9750b81): [`RaftStorage::append_to_log()`][] Modification**
    - **Description**: [`RaftStorage::append_to_log()`][] now accepts an `IntoIterator` instead of a slice.
    - **Upgrade Guide**: Update the method call to pass an `IntoIterator`.
    - **Example Code**: [examples/raft-kv-memstore/src/store/mod.rs](https://github.com/datafuselabs/openraft/commit/285e6225d3e0e8363d8b194e09a113f4e9750b81#diff-9b12e48c494438e11e2c13231c868015e735d0485badaec990ae4d9aad5309a4)
      ```diff
       impl RaftStorage<YourTypeConfig> for YourStore {
      -    async fn append_to_log(&mut self, entries: &[Entry<YourTypeConfig>]) -> Result<(), StorageError<_>> {
      +    async fn append_to_log<I>(&mut self, entries: I) -> Result<(), StorageError<_>>
      +    where I: IntoIterator<Item = Entry<YourTypeConfig>> + Send {
               let mut log = self.log.write().await;
               for entry in entries {
      -            log.insert(entry.log_id.index, (*entry).clone());
      +            log.insert(entry.log_id.index, entry);
               }
               Ok(())
           }
      ```

1. **Change [e0569988](https://github.com/datafuselabs/openraft/commit/e05699889e18e45eeb021ffdf8ca7924d8c218a3): Removal of `RaftStorageDebug`**
    - **Description**: The unused trait `RaftStorageDebug` has been removed.
    - **Upgrade Guide**: Remove any usage or implementation of `RaftStorageDebug`.

1. **Change [88f947a6](https://github.com/datafuselabs/openraft/commit/88f947a663c14dbd41ec39eee1d8d472c38d5706): Removal of Defensive Check Utilities**
    - **Description**: Defensive check utilities have been removed, and most defensive checks are replaced with `debug_assert!`.
    - **Upgrade Guide**: Remove usage of `StoreExt` and `DefensiveCheck`.

1. **Change [eaf45dfa](https://github.com/datafuselabs/openraft/commit/eaf45dfa3542340fe52a9796108513637e31e521): Moving `RaftStateMachine` out of `RaftStorage`**
    - **Description**: `RaftStateMachine` is now an independent storage component, separate from the log store.
    - **Upgrade Guide**: Use an [`Adapter`][] to wrap `RaftStorage`. Note that it requires feature flag [`storage-v2`][]
    - **Example Code**:
      ```diff
      let store = MyRaftStorage::new();
      - Raft::new(..., store);
      + let (log_store, sm) = Adapter::new(store);
      + Raft::new(..., log_store, sm);
      ```

1. **Change [9f8ae43e](https://github.com/datafuselabs/openraft/commit/9f8ae43e868c6d8518449dc331d8ad45833ef5bc): Modification of `RaftMetrics.replication` Type**
    - **Description**: The type of [`RaftMetrics.replication`][] has been changed to `BTreeMap<NodeId, Option<LogId>>`.
    - **Upgrade Guide**: Replace usage of `RaftMetrics.replication.data().replication.get(node_id)` with `RaftMetrics.replication.get(node_id)`.
    - **Example Code**:
      ```diff
      - let replication_metrics = raft_metrics.replication.data().replication.get(node_id);
      + let replication_metrics = raft_metrics.replication.get(node_id);
      ```

1. **Change [84539cb0](https://github.com/datafuselabs/openraft/commit/84539cb03b95ad96875b58961b2d29d9268f2f41): Moving Snapshot Type Definition**
    - **Description**: The snapshot type definition has been moved from storage traits to [`RaftTypeConfig`][].
    - **Upgrade Guide**: Update generic type parameters in application types to pass compilation.
    - **Example Code**:
      ```diff
       openraft::declare_raft_types!(
           pub TypeConfig:
              D = Request,
              R = Response,
              NodeId = NodeId,
              Node = BasicNode,
              Entry = openraft::Entry<TypeConfig>
      +       SnapshotData = Cursor<Vec<u8>>
       );
      ```

1. **Change [e78bbffe](https://github.com/datafuselabs/openraft/commit/e78bbffe6abf9a7197d20a609c98de037b731906): Removal of Unused Error `CommittedAdvanceTooMany`**
    - **Description**: The error `CommittedAdvanceTooMany` has been removed.
    - **Upgrade Guide**: Do not use this error in your code.


[`StoreBuilder`]: crate::testing::StoreBuilder
[`RaftStorage::append_to_log()`]: crate::storage::RaftStorage::append_to_log
[`RaftTypeConfig`]: crate::RaftTypeConfig
[`Adapter`]: crate::storage::Adaptor
[`storage-v2`]: crate::docs::feature_flags
[`RaftMetrics.replication`]: crate::metrics::RaftMetrics::replication

