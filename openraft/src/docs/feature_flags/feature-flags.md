# Feature flags

By default openraft enables no features.

- `bench`: Enables benchmarks in unittest. Benchmark in openraft depends on the unstable feature
  `test` thus it can not be used with stable rust. In order to run the benchmark with stable
  toolchain, the unstable features have to be enabled explicitly with environment variable
  `RUSTC_BOOTSTRAP=1`.
  <br/><br/>

- `bt`:
  attaches backtrace to generated errors. This feature works ONLY with nightly rust, because it requires unstable feature `error_generic_member_access`.
  <br/><br/>

- `loosen-follower-log-revert`:
  Permit the follower's log to roll back to an earlier state without causing the leader to panic.
  Although log state reversion is typically seen as a bug, enabling it can be useful for testing or other special scenarios.
  For instance, in an even number nodes cluster, erasing a node's data and then rebooting it(log reverts to empty) will not result in data loss.

  **Do not use it unless you know what you are doing**.
  <br/><br/>

- `serde`: derives `serde::Serialize, serde::Deserialize` for type that are used
  in storage and network, such as `Vote` or `AppendEntriesRequest`.
  <br/><br/>

- `single-term-leader`: allows only one leader to be elected in each `term`.
  This is the standard raft policy, which increases election confliction rate
  but reduce `LogId`(`(term, node_id, index)` to `(term, index)`) size.
  Read more about how it is implemented in [`vote`](./vote.md)
  <br/><br/>

- `compat-07`: provides additional data types to build v0.7 compatible RaftStorage.

   ```toml
   compat-07 = ["compat", "single-term-leader", "serde", "dep:or07", "compat-07-testing"]
   compat-07-testing = ["dep:tempdir", "anyhow", "dep:serde_json"]
   ```
  <br/><br/>

- `storage-v2`: enables `RaftLogStorage` and `RaftStateMachine` as the v2 storage
  This is a temporary feature flag, and will be removed in the future, when v2 storage is stable.
  This feature disables `Adapter`, which is for v1 storage to be used as v2.
  V2 storage separates log store and state machine store so that log IO and state machine IO can be parallelized naturally.
  <br/><br/>

- `singlethreaded`: removes `Send` and `Sync` bounds from `AppData`, `AppDataResponse`, `RaftEntry`, `SnapshotData`
  and other types to force the  asynchronous runtime to spawn any tasks in the current thread.
  This is for any single-threaded application that never allows a raft instance to be shared among multiple threads.
  This feature relies on the `async_fn_in_trait` language feature that is officially supported from Rust 1.75.0.
  If the feature is enabled, affected asynchronous trait methods and associated functions no longer use `async_trait`.
  In order to use the feature, `AsyncRuntime::spawn` should invoke `tokio::task::spawn_local` or equivalents.
  <br/><br/>

- `tracing-log`: enables "log" feature in `tracing` crate, to let tracing events
  emit log record.
  See: [tracing doc: emitting-log-records](https://docs.rs/tracing/latest/tracing/#emitting-log-records)
