# Feature flags

By default openraft enables no features.

- `bt`: attaches backtrace to generated errors.

- `serde`: derives `serde::Serialize, serde::Deserialize` for type that are used
    in storage and network, such as `Vote` or `AppendEntriesRequest`.

- `single-term-leader`: allows only one leader to be elected in each `term`.
    This is the standard raft policy, which increases election confliction rate
    but reduce `LogId`(`(term, node_id, index)` to `(term, index)`) size.
    Read more about how it is implemented in [`vote`](./vote.md)

- `compat-07`: provides additional data types to build v0.7 compatible RaftStorage.

   ```
   compat-07 = ["compat", "single-term-leader", "serde", "dep:or07", "compat-07-testing"]
   compat-07-testing = ["dep:tempdir", "anyhow", "dep:serde_json"]
   ```

- `storage-v2`: enables `RaftLogStorage` and `RaftStateMachine` as the v2 storage
  This is a temporary feature flag, and will be removed in the future, when v2 storage is stable.
  This feature disables `Adapter`, which is for v1 storage to be used as v2.
  V2 storage separates log store and state machine store so that log IO and state machine IO can be parallelized naturally. 
  