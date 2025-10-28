# Read Operations

Read operations within a Raft cluster are guaranteed to be linearizable.

This ensures that for any two read operations,
`A` and `B`, if `B` occurs after `A` by wall clock time,
then `B` will observe the same state as `A` or any subsequent state changes made by `A`,
regardless of which node within the cluster each operation is performed on.

In Openraft `read_index` is the same as the `read_index` in the original raft paper.
Openraft also use `read_log_id` instead of `read_index`.

## Ensuring linearizability

To ensure linearizability, read operations must perform a [`get_read_linearizer(read_policy)`][`get_read_linearizer()`] operation on the leader before proceeding.

This method confirms that this node is the leader at the time of invocation by either:
- sending heartbeats to a quorum of followers,
- or using leadership lease,

depending on the `read_policy` [`ReadPolicy`].  The `read_policy` can be one of:

- [`ReadPolicy::ReadIndex`]: Provides strongest consistency guarantees by confirming
  leadership with a quorum before serving reads, but incurs higher latency due to network
  communication.
- [`ReadPolicy::LeaseRead`]: Uses leadership lease to avoid network round-trips, providing
  better performance but slightly weaker consistency guarantees (assumes minimal clock drift
  between nodes).

[`get_read_linearizer()`] returns a [`Linearizer`], containing `(read_log_id, last_applied_log_id)`:

- `read_log_id` represents the log id up to which the state machine should apply to ensure a
  linearizable read,
- `last_applied_log_id` is the last applied log id.

The caller then wait for `last_applied_log_id` to catch up `read_log_id`,
by calling [`Linearizer::try_await_ready()`], or [`Linearizer::await_ready()`] for simplicity without a timeout,
and at last, proceed with the state machine read.

The above steps are encapsulated in the [`ensure_linearizable(read_policy)`][`ensure_linearizable()`] method.

## Examples

```ignore
my_raft.ensure_linearizable(read_policy).await?;
proceed_with_state_machine_read();
```

The above snippet does the same as the following:

```ignore
let linearizer = my_raft.get_read_linearizer(ReadPolicy::ReadIndex).await?;

linearizer.await_ready(&my_raft, None).await?;

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
- [`/get_linearizer` endpoint](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/src/network/management.rs) - Leader provides linearizer data
- [`/follower_read` endpoint](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/src/network/api.rs) - Follower read implementation
- [Test coverage](https://github.com/databendlabs/openraft/blob/main/examples/raft-kv-memstore/tests/cluster/test_follower_read.rs)


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
[`Raft::metrics`]: crate::Raft::metrics
[`Linearizer`]: crate::raft::linearizable_read::Linearizer
[`Linearizer::await_ready()`]: crate::raft::linearizable_read::Linearizer::await_ready
[`Linearizer::try_await_ready()`]: crate::raft::linearizable_read::Linearizer::try_await_ready
[`ReadPolicy`]: crate::raft::ReadPolicy
[`ReadPolicy::ReadIndex`]: crate::raft::ReadPolicy::ReadIndex
[`ReadPolicy::LeaseRead`]: crate::raft::ReadPolicy::LeaseRead
