
By default openraft enables no features.

## feature-flag `bench`

Enables benchmarks in unittest. Benchmark in openraft depends on the unstable feature
`test` thus it can not be used with stable rust. In order to run the benchmark with stable
toolchain, the unstable features have to be enabled explicitly with environment variable
`RUSTC_BOOTSTRAP=1`.

## feature-flag `bt`

attaches backtrace to generated errors.
This feature works ONLY with nightly rust, because it requires unstable feature `error_generic_member_access`.

## feature-flag `compat`

Enables compatibility supporting types.

## feature-flag `generic-snapshot-data`

Enable this feature flag
to eliminate the `AsyncRead + AsyncWrite + AsyncSeek + Unpin` bound
from [`RaftTypeConfig::SnapshotData`](crate::RaftTypeConfig::SnapshotData)
Enabling this feature allows applications to use a custom snapshot data format and transport fragmentation,
diverging from the default implementation which typically relies on a single-file structure.

By default, it is off.
This feature is introduced in 0.9.0

On the sending end (leader that sends snapshot to follower):

- Without `generic-snapshot-data`: [`RaftNetwork::full_snapshot()`]
  provides a default implementation that invokes the chunk-based API
  [`RaftNetwork::install_snapshot()`] for transmit.

- With `generic-snapshot-data` enabled: [`RaftNetwork::full_snapshot()`]
  must be implemented to provide application customized snapshot transmission.
  Application does not need to implement [`RaftNetwork::install_snapshot()`].

On the receiving end(follower):

- `Raft::install_snapshot()` is available only when `generic-snapshot-data` is disabled.

Refer to example `examples/raft-kv-memstore-generic-snapshot-data` with `generic-snapshot-data` enabled.

## feature-flag `loosen-follower-log-revert`

Permit the follower's log to roll back to an earlier state without causing the leader to panic.
Although log state reversion is typically seen as a bug, enabling it can be useful for testing or other special scenarios.
For instance, in an even number nodes cluster,
erasing a node's data and then rebooting it(log reverts to empty) will not result in data loss.

**Do not use it unless you know what you are doing**.

## feature-flag `serde`

Derives `serde::Serialize, serde::Deserialize` for type that are used
in storage and network, such as `Vote` or `AppendEntriesRequest`.

## feature-flag `single-term-leader`

Allows only one leader to be elected in each `term`.
This is the standard raft policy, which increases election conflict rate
but reduce `LogId` size(`(term, node_id, index)` to `(term, index)`).

Read more about how it is implemented in:
[`leader_id`](crate::docs::data::leader_id)
and [`vote`](crate::docs::data::vote).

## feature-flag `singlethreaded`

Removes `Send` and `Sync` bounds from `AppData`, `AppDataResponse`, `RaftEntry`, `SnapshotData`
and other types to force the  asynchronous runtime to spawn any tasks in the current thread.
This is for any single-threaded application that never allows a raft instance to be shared among multiple threads.
This feature relies on the `async_fn_in_trait` language feature that is officially supported from Rust 1.75.0.
If the feature is enabled, affected asynchronous trait methods will not require `Send` bounds.
In order to use the feature, `AsyncRuntime::spawn` should invoke `tokio::task::spawn_local` or equivalents.

## feature-flag `storage-v2`

Enables `RaftLogStorage` and `RaftStateMachine` as the v2 storage
This is a temporary feature flag, and will be removed in the future, when v2 storage is stable.
This feature disables `Adapter`, which is for v1 storage to be used as v2.
V2 storage separates log store and state machine store so that log IO and state machine IO can be parallelized naturally.

## feature-flag `tracing-log`

Enables "log" feature in `tracing` crate, to let tracing events
emit log record.
See: [tracing doc: emitting-log-records](https://docs.rs/tracing/latest/tracing/#emitting-log-records)


[`RaftNetwork::full_snapshot()`]: crate::network::RaftNetwork::full_snapshot
[`RaftNetwork::install_snapshot()`]: crate::network::RaftNetwork::install_snapshot