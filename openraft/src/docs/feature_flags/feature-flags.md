
By default openraft enables `["tokio-rt"]`.


## feature-flag `adapt-network-v1` (removed)

This feature flag has been removed since `0.10.0`.

For backward compatibility with the v1 `RaftNetwork` trait and chunk-based
snapshot transport, use the [`openraft-network-v1`] crate instead.

[`openraft-network-v1`]: https://crates.io/crates/openraft-network-v1


## feature-flag `bench`

Enables benchmarks in unittest. Benchmark in openraft depends on the unstable feature
`test` thus it cannot be used with stable rust. In order to run the benchmark with stable
toolchain, the unstable features have to be enabled explicitly with environment variable
`RUSTC_BOOTSTRAP=1`.

## feature-flag `bt`

attaches backtrace to generated errors.
This feature works ONLY with nightly rust, because it requires unstable feature `error_generic_member_access`.

## feature-flag `compat`

Enables compatibility supporting types.


## feature-flag `runtime-stats`

**Unstable**: This feature is experimental and the API may change in future versions.

Exposes the `stats` module and adds the `Raft::runtime_stats()` method
for accessing runtime statistics.

When enabled, the `stats` module provides:
- `Histogram`: Tracks distribution of values in logarithmic buckets
- `RuntimeStats`: Contains histograms for apply and append batch sizes

Example usage:
```rust,ignore
use openraft::stats::RuntimeStats;

let raft = Raft::new(...).await?;
let stats: RuntimeStats = raft.runtime_stats().await?;
println!("Total applies: {}", stats.apply_batch.total());
println!("P99 batch size: {:?}", stats.apply_batch.percentile(0.99));
```


## feature-flag `serde`

Derives `serde::Serialize, serde::Deserialize` for type that are used
in storage and network, such as `Vote` or `AppendEntriesRequest`.

## feature-flag `single-term-leader`

**This feature flag is removed**.
Use [`leader_id_std::LeaderId`] in [`RaftTypeConfig`] instead.

Allows only one leader to be elected in each `term`.
This is the standard raft policy, which increases election conflict rate
but reduce `LogId` size(`(term, node_id, index)` to `(term, index)`).

Read more about how it is implemented in:
[`leader_id`](crate::docs::data::leader_id)
and [`vote`](crate::docs::data::vote).


## feature-flag `single-threaded`

Removes `Send` and `Sync` bounds from `AppData`, `AppDataResponse`, `RaftEntry`, `SnapshotData`
and other types to force the asynchronous runtime to spawn any tasks in the current thread.
This is for any single-threaded application that never allows a raft instance to be shared among multiple threads.
This feature relies on the `async_fn_in_trait` language feature that is officially supported from Rust 1.75.0.
If the feature is enabled, affected asynchronous trait methods will not require `Send` bounds.
In order to use the feature, `AsyncRuntime::spawn` should invoke `task::spawn_local` or equivalents.


## feature-flag `tokio-rt`

Using `tokio` as the default runtime implementation.
With this feature disabled, application should implement and set the
async-runtime to `AsyncRuntime` manually in [`RaftTypeconfig`] implementation.


## feature-flag `tracing-log`

Enables "log" feature in `tracing` crate, to let tracing events
emit log record.
See: [tracing doc: emitting-log-records](https://docs.rs/tracing/latest/tracing/#emitting-log-records)


## feature-flag `type-alias`

Enable this feature to use type shortcuts defined in `openraft::alias::*`

For example:
```rust,ignore
use openraft::alias::SnapshotDataOf;

struct MyTypeConfig;
impl RaftTypeconfig For MyTypeConfig { /*...*/ }

// The following two lines are equivalent:
let snapshot_data: SnapshotDataOf<MyTypeConfig>;
let snapshot_data: <MyTypeConfig as RaftTypeConfig>::SnapshotData;
```

Note that the type shortcuts are not stable and may be changed in the future.
It is also a good idea to copy the type shortcuts to your own codebase if you
want to use them.

[`RaftTypeConfig`]: crate::RaftTypeConfig
[`leader_id_std::LeaderId`]: crate::impls::leader_id_std::LeaderId
