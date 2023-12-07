# Read Operations

Read operations within a Raft cluster are guaranteed to be linearizable.

This ensures that for any two read operations,
`A` and `B`, if `B` occurs after `A` by wall clock time,
then `B` will observe the same state as `A` or any subsequent state changes made by `A`,
regardless of which node within the cluster each operation is performed on.

In Openraft `read_index` is the same as the `read_index` in the original raft paper.
Openraft also use `read_log_id` instead of `read_index`.

## Ensuring linearizability

To ensure linearizability, read operations must perform a [`get_read_log_id()`] operation on the leader before proceeding.

This method confirms that this node is the leader at the time of invocation by sending heartbeats to a quorum of followers, and returns `(read_log_id, last_applied_log_id)`:
- `read_log_id` represents the log id up to which the state machine should apply to ensure a
  linearizable read,
- `last_applied_log_id` is the last applied log id.

The caller then wait for `last_applied_log_id` to catch up `read_log_id`, which can be done by subscribing to [`Raft::metrics`],
and at last, proceed with the state machine read.

The above steps are encapsulated in the [`ensure_linearizable()`] method.

## Examples

```ignore
my_raft.ensure_linearizable().await?;
proceed_with_state_machine_read();
```

The above snippet does the same as the following:

```ignore
let (read_log_id, applied) = self.get_read_log_id().await?;

if read_log_id.index() > applied.index() {
    self.wait(None).applied_index_at_least(read_log_id.index(), "").await?
}

proceed_with_state_machine_read();
```

The comparison `read_log_id > applied_log_id` would also be valid in the above example.


## Ensuring Linearizability with `read_log_id`

The `read_log_id` is determined as the maximum of the `last_committed_log_id` and the
log id of the first log entry in the current leader's term (the "blank" log entry).

Assumes another earlier read operation reads from state machine with up to log id `A`.
Since the leader has all committed entries up to its initial blank log entry,
we have: `read_log_id >= A`.

When the `last_applied_log_id` meets or exceeds `read_log_id`,
the state machine contains all state upto `A`. Therefore, a linearizable read is assured
when `last_applied_log_id >= read_log_id`.


## Ensuring Linearizability with `read_index`

And it is also legal by comparing `last_applied_log_id.index() >= read_log_id.index()`
due to the guarantee that committed logs will not be lost.

Since a previous read could only have observed committed logs, and `read_log_id.index()` is
at least as large as any committed log, once `last_applied_log_id.index() >= read_log_id.index()`, the state machine is assured to reflect all entries seen by any past read.


[`ensure_linearizable()`]: crate::Raft::ensure_linearizable
[`get_read_log_id()`]: crate::Raft::get_read_log_id
[`Raft::metrics`]: crate::Raft::metrics
