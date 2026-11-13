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

Both [`get_read_linearizer()`] and [`get_local_read_linearizer()`] return a [`Linearizer`],
containing `(read_log_id, last_applied_log_id)`:

- `read_log_id` represents the log id up to which the state machine should apply to ensure a
  linearizable read,
- `last_applied_log_id` is the last applied log id.

The caller then wait for `last_applied_log_id` to catch up `read_log_id`,
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

## Opt-in relaxed leader reads

[`Raft::read_relaxed`] reduces apply waiting by using the authorizing leadership's no-op log ID
rather than its committed tail. Enable it with `Config::enable_relaxed_read: Some(true)`;
`None` (the default) and `Some(false)` return `RelaxedReadError::Disabled`. Existing read APIs
retain their committed boundary and transferable follower-read barriers.

This optimization requires **leader-only linearizable reads across the application**. A follower
can apply a committed entry before the leader applies it. If it returns that state, a later
relaxed read on the leader could return older state. Even ordinary follower reads using the
standard committed barrier must therefore be stopped and drained before enabling relaxed reads
anywhere in the cluster. Every leader read path must also use `read_relaxed` or equivalent
sampled-leadership validation: an ordinary leader read authorized before step-down can otherwise
expose the next leader's state while finishing as a follower. This applies even on nodes where
relaxed reads are disabled. Writes must respond only after their state-machine updates are applied.

A relaxed read still confirms leadership using the requested ReadIndex or lease options and
waits for the current leadership's no-op to be applied. Previously completed local writes are
already applied; previous leaders' writes precede the new no-op. Unfinished overlapping writes
can be ordered after the read. `enable_local_lease_read` independently enables the cached lease
path; disabling it makes `read_relaxed` use RaftCore without disabling the reduced apply barrier.

### Step-down counterexample

A no-op barrier alone is unsafe. A read can outlive the leadership that authorized it.

Suppose `x = 0`. A leads in leadership `T`. Its no-op is at log index `10`, and it has applied
through index `20`. No write resets `x` during this example. The following schedule assumes
relaxed reads **without snapshot validation**:

1. A authorizes read `R_A` in `T`. Its no-op barrier at `10` is already applied. The read task
   pauses before querying the state machine.
2. A steps down. B becomes leader in `T+1`. B commits and applies its no-op at index `21`.
3. B commits entry `22`, which sets `x = 1`. B's apply worker pauses before this entry.
   B has `committed = 22`, `applied = 21`, and `x = 0`. The write response is still pending.
4. B sends A entry `22` and its commit information. A applies it as a follower. A now has
   `applied = 22` and `x = 1`. Followers need not wait for the leader's state machine to apply.
5. `R_A` resumes on A, samples `x = 1`, and returns it to the client.
6. Only after receiving that response, the client invokes read `R_B` on B. B confirms leadership.
   Its relaxed barrier is no-op `21`, which is already applied.
7. B has not applied entry `22`. `R_B` returns `x = 0`. Applying `22` later cannot repair this
   read regression.

Time runs downward. The numbers below are log indexes, not terms:

```text
Time   Client                 A                                  B
----   --------------------   --------------------------------   ----------------------------
t1     Invoke R_A ----------> Authorize in T; barrier=10
                              Pause before sampling

t2                            Step down; become follower         Become leader T+1
                                                                 Apply no-op 21; x=0

t3                                                               Commit 22: x=1
                                                                 Keep applied=21; x=0
                                                                 Write response pending

t4                            Receive entry 22 and commit
                              Apply 22 as follower; x=1

t5     Receive x=1 <--------- Resume R_A; sample and return 1

t6     Invoke R_B ---------------------------------------------> Confirm leadership; barrier=21
                                                                 Barrier already applied

t7     Receive x=0 <-------------------------------------------- Sample and return 0
```

The reads do not overlap: `response(R_A) < invocation(R_B)`. Linearizability requires `R_A`
before `R_B`. Observing `x = 1` places the pending write before `R_A`, so `R_B` cannot return `0`
without an intervening reset. The pending write response does not change that requirement.
The leadership change overlaps `R_A`; the two reads do not overlap each other.

Both requests were sent to leaders. The first query sampled state as a follower only because
it survived step-down. Routing requests only to leaders is therefore insufficient.

### Why ordinary local linearizable reads do not regress here

[`get_local_read_linearizer()`] retains the `max(noop_log_id, committed)` barrier. In this
schedule, B's boundary for `R_B` is `22`, not `21`. The caller must await the linearizer before
querying. Since B has applied only through `21`, the read waits for `22`, then observes `x = 1`.
RaftCore publishes committed progress before exposing the corresponding effects, so the local
path cannot authorize this later read using an outdated published boundary of `21`.
[`get_read_linearizer()`] and [`ensure_linearizable()`] retain the same barrier.

The distinction is the apply barrier, not whether the read enters RaftCore's queue. An ordinary
read on A can still survive step-down and expose entry `22`. If B's later read is relaxed, the
counterexample applies again. This is why all read paths need compatible snapshot validation
when relaxed reads are enabled.

### Snapshot validation

[`Raft::read_relaxed`] encompasses the query so it can reject unsafe results. In the example,
A's sampled log ID `22` belongs to `T+1`, not its authorizing leadership `T`. The API discards
that result instead of returning `x = 1`.

The query must acquire a fresh, coherent snapshot of this node's state machine and return
`(applied_log_id, value)` from that same snapshot. It must not use metrics for the applied log ID,
reuse an older transaction snapshot, or expose the value before the API succeeds. The sampled
applied log ID must cover the authorized no-op and have the same committed leadership identity.
A mismatch discards the value and returns `RelaxedReadError::InvalidSnapshot`; retry the whole
read with fresh authorization. Queries are invoked at most once and must be read-only.

```ignore
let value = raft.read_relaxed(ReadPolicy::LeaseRead, || async {
    let state = local_state_machine.read().await;
    (state.last_applied_log.clone(), state.get("key").cloned())
}).await?;
```

The result is a validated query value, not a transferable read barrier. This API deliberately
provides no relaxed follower-read token.

Further reading:

- [ReadIndex: Less Latency through Relaxed Ordering](https://github.com/drmingdrmer/consensus-essence/blob/main/src/list/raft-read-index-relaxed-order/raft-read-index-relaxed-order.md)
  explains the optimization and the follower-read restriction.
- [Issue #1267](https://github.com/databendlabs/openraft/issues/1267) contains the proposal and
  discussion of the step-down race that sampled-leadership validation prevents.
- [The minimal benchmark README](https://github.com/databendlabs/openraft/blob/main/benchmarks/minimal/README.md#comparing-core-local-and-relaxed-reads)
  documents the three-way query-inclusive comparison and its measurement limitations.

[`Raft::read_relaxed`]: crate::Raft::read_relaxed

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
