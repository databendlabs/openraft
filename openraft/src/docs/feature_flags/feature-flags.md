
By default openraft enables `["tokio-rt", "clap"]`.


## feature-flag `adapt-network-v1` (removed)

This feature flag has been removed since `0.10.0`.

For backward compatibility with the v1 `RaftNetwork` trait and chunk-based
snapshot transport, use the [`openraft-legacy`] crate instead.

[`openraft-legacy`]: https://crates.io/crates/openraft-legacy


## feature-flag `bench`

Enables benchmarks in unittest. Benchmark in openraft depends on the unstable feature
`test` thus it cannot be used with stable rust. In order to run the benchmark with stable
toolchain, the unstable features have to be enabled explicitly with environment variable
`RUSTC_BOOTSTRAP=1`.

## feature-flag `bt`

attaches backtrace to generated errors.
This feature requires nightly Rust on older toolchains due to the `error_generic_member_access` feature, which has been stabilized in recent nightly versions.


## feature-flag `clap`

Enables building [`Config`] from command-line arguments via
[`Config::build`], and derives [`clap::Parser`] on `Config`.
Pulls in `clap` and `byte-unit` (the latter only used to parse byte
sizes like `3MiB` for `Config::snapshot_max_chunk_size`).

Enabled by default. When disabled, `Config` must be constructed
programmatically using [`Config::default`] and struct-update syntax:

```rust,ignore
use openraft::Config;

let config = Config {
    cluster_name: "my-cluster".into(),
    election_timeout_min: 200,
    ..Default::default()
};
```

[`Config`]: crate::Config
[`Config::build`]: crate::Config::build
[`Config::default`]: crate::Config::default
[`clap::Parser`]: https://docs.rs/clap/latest/clap/trait.Parser.html


## feature-flag `compat`

Enables compatibility supporting types.


## feature-flag `loosen-follower-log-revert` (removed)

This feature flag has been removed since `0.10.0`.

Use [`Config::allow_log_reversion`] instead to allow a follower's log to revert
to an earlier state.

[`Config::allow_log_reversion`]: crate::Config::allow_log_reversion


## feature-flag `metrics-logids`

Includes `LogIdList` in [`RaftMetrics`] and [`RaftDataMetrics`].
This is primarily used for deterministic testing, where the exact per-leader
log state needs to be observable through metrics.

Since `LogIdList` is heap-allocated, it is gated behind this feature flag
to avoid overhead in production builds.

[`RaftMetrics`]: crate::metrics::RaftMetrics
[`RaftDataMetrics`]: crate::metrics::RaftDataMetrics


## feature-flag `runtime-stats`

**Unstable**: This feature is experimental and the API may change in future versions.

Exposes the `stats` module and adds the `Raft::runtime_stats()` method
for accessing runtime statistics.

When enabled, the `stats` module provides:
- `Histogram`: Tracks distribution of values in logarithmic buckets
- `RuntimeStats`: Contains histograms for apply and append batch sizes, plus
  lifecycle latency histograms

Example usage:
```rust,ignore
use openraft::stats::RuntimeStats;

let raft = Raft::new(...).await?;
let stats: RuntimeStats = raft.runtime_stats().await?;
println!("Total applies: {}", stats.apply_batch.total());
println!("P99 batch size: {:?}", stats.apply_batch.percentile(0.99));
```

Tracks per-entry latency across 6 lifecycle stages on the leader node:

- **proposed**: when the application called `client_write()`
- **received**: when `RaftCore` dequeued the request from the API channel
- **submitted**: when the entry was submitted to Raft-Log storage
- **persisted**: when storage confirmed persistence (append callback returned)
- **committed**: when this node locally marked the entry as committed (local
  commit timestamp, not the cluster-wide quorum time)
- **applied**: when the state machine finished applying the entry and reported
  the result back to `RaftCore`

Each stage records timestamps in a fixed-capacity ring buffer (configurable via
`Config::log_stage_capacity`), enabling stage-to-stage duration histograms
that reveal where latency accumulates (channel queueing, storage I/O, replication,
and state machine apply time).

The `RuntimeStats::lifecycle_latency` field
contains computed `LifecycleLatencyHistograms` with percentile distributions for
each stage transition.


## feature-flag `serde`

Derives `serde::Serialize, serde::Deserialize` for type that are used
in storage and network, such as `Vote` or `AppendEntriesRequest`.

## feature-flag `singlethreaded` (removed)

This feature flag has been renamed to `single-threaded` since `0.10.0`.

Please update your `Cargo.toml` to use `single-threaded` instead.


## feature-flag `single-term-leader` (removed)

This feature flag has been removed since `0.10.0`.
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
