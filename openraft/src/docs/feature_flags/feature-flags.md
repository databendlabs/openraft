
By default openraft enables `["tokio-rt", "adapt-network-v1"]`.


## feature-flag `adapt-network-v1`

When implementing [`RaftNetworkV2<T>`][`RaftNetworkV2`] for a generic type parameter `T`, you might
encounter a compiler error about conflicting implementations. This happens
because Openraft provides a blanket implementation that adapts `RaftNetwork`
implementations to [`RaftNetworkV2`]. For example:

```rust,ignore
pub trait RaftTypeConfigExt: openraft::RaftTypeConfig {}
pub struct YourNetworkType {}
impl<T: RaftTypeConfigExt> RaftNetworkV2<T> for YourNetworkType {}
```

You might encounter the following error:

```text
conflicting implementations of trait `RaftNetworkV2<_>` for type `YourNetworkType`
conflicting implementation in crate `openraft`:
- impl<C, V1> RaftNetworkV2<C> for V1
```

If you encounter this error, you can disable the feature `adapt-network-v1` to
remove the default implementation for [`RaftNetworkV2`].

[`RaftNetworkV2`]: crate::network::v2::RaftNetworkV2


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


## feature-flag `singlethreaded`

Removes `Send` and `Sync` bounds from `AppData`, `AppDataResponse`, `RaftEntry`, `SnapshotData`
and other types to force the asynchronous runtime to spawn any tasks in the current thread.
This is for any single-threaded application that never allows a raft instance to be shared among multiple threads.
This feature relies on the `async_fn_in_trait` language feature that is officially supported from Rust 1.75.0.
If the feature is enabled, affected asynchronous trait methods will not require `Send` bounds.
In order to use the feature, `AsyncRuntime::spawn` should invoke `tokio::task::spawn_local` or equivalents.


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
