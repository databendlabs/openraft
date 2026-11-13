# Read Operations

Read operations within a Raft cluster are guaranteed to be linearizable.

This ensures that for any two read operations,
`A` and `B`, if `B` occurs after `A` by wall clock time,
then `B` will observe the same state as `A` or any subsequent state changes made by `A`,
regardless of which node within the cluster each operation is performed on.

In Openraft `read_index` is the same as the `read_index` in the original raft paper.
Openraft also use `read_log_id` instead of `read_index`.

## Ensuring linearizability

To ensure linearizability, obtain a [`Linearizer`] on the leader with
[`get_read_linearizer()`] or [`get_local_read_linearizer()`] before proceeding.

Both methods authorize a read using either:
- sending heartbeats to a quorum of followers,
- or using leadership lease,

depending on the `read_policy` [`ReadPolicy`].  The `read_policy` can be one of:

- [`ReadPolicy::ReadIndex`]: Confirms leadership with a fresh quorum acknowledgement before
  serving reads, incurring network latency.
- [`ReadPolicy::LeaseRead`]: Reuses a fresh quorum acknowledgement to avoid network round-trips.
  Its consistency guarantee is slightly weaker than `ReadIndex`, because it assumes that clock
  drift between nodes is bounded. A follower that restarts honors its leader's lease for a full
  lease period after the restart: until then, it neither votes for another candidate nor
  campaigns.

Both [`get_read_linearizer()`] and [`get_local_read_linearizer()`] return a [`Linearizer`]
that retains the committed log ID and the authorizing leadership's no-op log ID separately.
Its `read_log_id()` calculates their maximum: the state machine must apply through that boundary
before reading. Applied progress is tracked with the node from which it was collected; it is
not a snapshot of the subsequent query.

The caller then waits for applied progress to catch up to `read_log_id`,
by calling [`Linearizer::try_await_ready()`], or [`Linearizer::await_ready()`] for simplicity without a timeout,
and at last, proceed with the state machine read.

The above steps are encapsulated in the [`ensure_linearizable(read_policy)`][`ensure_linearizable()`] method.

## Caller-side lease reads

Choose the API explicitly:

| API | Request handling |
| --- | --- |
| [`get_read_linearizer()`], [`get_read_log_id()`], [`ensure_linearizable()`] | Always enqueue the read request in RaftCore's API queue alongside writes. |
| [`get_local_read_linearizer()`] | Try the published leader snapshot in the caller's task when enabled; otherwise enqueue through `get_read_linearizer()`. |

Set [`Config::enable_local_lease_read`] to `Some(true)` to enable the caller-side check in
`get_local_read_linearizer()`. With `None` (the default) or `Some(false)`, that method uses
`get_read_linearizer()` with the same options. The existing read APIs always use RaftCore.

On a cache hit, the local method uses the published committed boundary without entering the
core's queue. It can therefore complete before an earlier queued write request is handled,
even while the core is busy. A core read uses the committed boundary when RaftCore handles it.
Queue order serializes request handling, not write completion: an earlier write can still be
uncommitted when the core handles a read. Both APIs support linearizable reads, and an unfinished
write that overlaps a read may follow that read. Await the write response before starting a read
that must observe it.

The caller-side lease check uses acknowledged RPC sending time and caps the accepted age at the
configured leader lease. A missing or expired snapshot falls back to the core, preserving the
options' forwarding, heartbeat, and timeout behavior. `ReadIndex` and zero effective acknowledgement
age always use RaftCore for fresh quorum confirmation.

RaftCore publishes committed progress before dispatching apply work and invalidates the publication before leadership transfer.
Each read loads the current publication for that invocation.
A read that captured a valid snapshot before invalidation overlaps the transfer and may finish afterward.

An immediate cache hit authorizes the read; application state may still need to catch up.
The returned `Linearizer` retains its apply barrier:

```ignore
let linearizer = my_raft.get_local_read_linearizer(ReadPolicy::LeaseRead).await?;
linearizer.await_ready(&my_raft).await?;
proceed_with_state_machine_read();
```

The apply wait uses the applied log id in [`Raft::metrics`], published once per RaftCore loop iteration.
If that progress is behind the read boundary, the wait depends on RaftCore handling the apply result
and publishing metrics, even when the state machine has already applied the entry.

In a read-only benchmark with three in-process voters, local reads reached 9.14 million reads/s
versus 2.35 million through RaftCore with 64 readers (3.9×).
Single-reader p50 latency fell from 9.17 µs to 0.083 µs.
These medians of five release runs used an Apple M3 Max, in-memory storage, and one client thread,
and included the apply barrier. At a paced 180,000 reads/s plus 20,000 writes/s, both paths had
similar read p99 latency of about 2.12 ms, including pacing delay.

The [minimal benchmark](https://github.com/databendlabs/openraft/blob/main/benchmarks/minimal/README.md#lease-reads)
compares both paths and explains what its numbers do and do not measure.

## Examples

```ignore
my_raft.ensure_linearizable(read_policy).await?;
proceed_with_state_machine_read();
```

The above snippet does the same as the following:

```ignore
let linearizer = my_raft.get_read_linearizer(ReadPolicy::ReadIndex).await?;

linearizer.await_ready(&my_raft).await?;

// Following read from state machine is linearized across the cluster
let val = my_raft.with_state_machine(|sm| { sm.read("foo") }).await?;
```

The above snippet shows how to perform a linearizable read on the leader.

### Follower Read

It is also possible to perform linearizable reads on a follower so that the read load is distributed across the cluster.
In this scenario:

1. The follower obtains the `read_log_id` from the leader via application-defined RPC
2. The follower waits for its local state machine to apply up to the `read_log_id`
3. The follower can then safely read from its local state machine

Note that in this scenario, the application is responsible for implementing the RPC mechanism to obtain the read log ID from the leader.

A basic example of a follower read:

```ignore
// Get read_log_id from remote leader via RPC
let leader_id = my_raft.current_leader().await?.unwrap();
let linearizer = my_app_rpc.get_read_linearizer(leader_id, ReadPolicy::ReadIndex).await?;

// Block waiting local state machine to apply up to to the `read_log_id`
let linearized_state = linearizer.try_await_ready(&my_raft, None).await?.unwrap();

// Following read from state machine is linearized across the cluster
let val = my_raft.with_state_machine(|sm| { sm.read("foo") }).await?;
```

See a complete implementation in the [raft-kv-memstore example](https://github.com/databendlabs/openraft/tree/main/examples/raft-kv-memstore):
- [`/get_linearizer` endpoint](https://github.com/databendlabs/openraft/blob/main/examples/app-http/src/app.rs) - `App::get_linearizer()`, where the leader provides linearizer data
- [`/follower_read` endpoint](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/src/http_api.rs) - Follower read implementation
- [Test coverage](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/tests/cluster/test_read_modes.rs)


## Ensuring Linearizability with `read_log_id`

The `read_log_id` is determined as the maximum of the `last_committed_log_id` and the
noop log id, i.e., the log id of the first log entry in the current leader's term (the "blank" log entry).

Assumes another earlier read operation reads from state machine with up to log id `A`.
Since the leader has all committed entries up to its initial blank log entry,
we have: `read_log_id >= A`.

When the `last_applied_log_id` meets or exceeds `read_log_id`,
the state machine contains all state up to `A`. Therefore, a linearizable read is assured
when `last_applied_log_id >= read_log_id`.


## Ensuring Linearizability with `read_index` or `lease_read`

And it is also legal by comparing `last_applied_log_id.index() >= read_log_id.index()`
due to the guarantee that committed logs will not be lost.

Since a previous read could only have observed committed logs, and `read_log_id.index()` is
at least as large as any committed log, once `last_applied_log_id.index() >= read_log_id.index()`, the state machine is assured to reflect all entries seen by any past read.


[`ensure_linearizable()`]: crate::Raft::ensure_linearizable
[`get_read_linearizer()`]: crate::Raft::get_read_linearizer
[`get_local_read_linearizer()`]: crate::Raft::get_local_read_linearizer
[`get_read_log_id()`]: crate::Raft::get_read_log_id
[`Raft::metrics`]: crate::Raft::metrics
[`Linearizer`]: crate::raft::linearizable_read::Linearizer
[`Linearizer::await_ready()`]: crate::raft::linearizable_read::Linearizer::await_ready
[`Linearizer::try_await_ready()`]: crate::raft::linearizable_read::Linearizer::try_await_ready
[`ReadPolicy`]: crate::raft::ReadPolicy
[`ReadPolicy::ReadIndex`]: crate::raft::ReadPolicy::ReadIndex
[`ReadPolicy::LeaseRead`]: crate::raft::ReadPolicy::LeaseRead

[`Config::enable_local_lease_read`]: crate::Config::enable_local_lease_read
