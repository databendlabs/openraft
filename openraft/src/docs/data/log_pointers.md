# Understanding Openraft Log Pointers

Inside Openraft, there are several pointers pointing to the raft-log:

- `purged`:    the last log entry that has been purged from the log, inclusive.
  Log entries that are included in a snapshot will be purged.
  openraft purges logs from the beginning to a configured position, when a fresh snapshot is built,
  `purged` is the greatest log id that openraft **does not store**.

- `snapshot`:  the last log entry that has been included in the latest snapshot, inclusive.
  Openraft takes a snapshot of the state-machine periodically.
  Therefore, a snapshot is a compacted format of a continuous range of log entries.

- `applied`:   the last log entry that has been applied to the state machine, inclusive.
  In raft only committed log entry can be applied.

- `committed`: the last log entry that has been committed (granted by a quorum and at the leader's term) by the leader, inclusive.

- `flushed`:  the last log entry that is persisted on disk, inclusive.

- `submitted`: the last log entry that has submitted to [`RaftLogStorage`], but not yet flushed, inclusive.

- `accepted`: the last log entry accepted from the Leader but not yet submit to the storage, inclusive.
  `accepted` is also known as `last_log` in Openraft codebase.

Within openraft, the log pointers mentioned above always hold that:
`flushed` ≤ `submitted` ≤ `accepted`;
`purged` ≤ `snapshot` ≤ `applied` ≤ `committed` ≤ `submitted`;

- `committed` ≤ `flushed` does not have to hold,
  because `committed` means a quorum flushed the log entry,
  but the quorum does not have to include the local node.

- `committed` ≤ `submitted` always hold
  only because the replication mod needs to read log entries from local storage.

Invariants:

```text
                               RaftLogStorage
.----------------------------------------------------------------------------.
| purged ≤ -.                             flushed ≤ -+- submitted ≤ accepted |
'-----------|----------------------------------------|-----------------------'
            |                                        |
            |                        .- committed ≤ -'
            |                        |
          .-|------------------------|-.
          | '- snapshot ≤ applied ≤ -' |
          '----------------------------'
                 RaftStateMachine
```

The log indexes of a follower are always behind or equal the leader's `last_log`.
If a follower's `last_log` is lagging behind the leader's `purged`, it will be replicated using a snapshot.

```text
ø: inexistent log entry
L: existent log entry
P: existent and persisted log entry

     .- delayed follower
     |
øøøøøøøøøøøøPPPPPPPPPPPPPPPPPPPPPPPPLLLLLLLLLLLL
-----+-----+-----+-----+-----+-----+-----+-----+-----------> log index
           |     |     |     |     |     |     ` accepted
           |     |     |     |     |     ` submitted
           |     |     |     |     ` flushed
           |     |     |     ` committed
           |     |     ` applied
           |     ` snapshot
           ` purged
```

## Optionally Persisted `committed`

In standard raft, `committed` is not persisted.
`committed` log index can be recovered when leader established (by re-commit all logs),
or when a follower receives `committed` log id message from the leader.

Openraft provides optional API for application to store `committed` log id.
See: [`RaftLogStorage::save_committed`].

If the state machine does not flush state to disk before returning from `apply()`,
[`RaftLogStorage::save_committed`] may help restoring the last committed log id.

- If the `committed` log id is saved, the state machine will be recovered to the state
  corresponding to this `committed` log id upon system startup, i.e., the state at the point
  when the committed log id was applied.

- If the `committed` log id is not saved, Openraft will just recover the state machine to
  the state of the last snapshot taken.

### Reverted reads when `committed` is not saved

If `committed` is not saved, the state machine is recovered only to the last snapshot on
restart. It catches back up once this node perceives the cluster commit re-established by the
current leader and re-applies up to it; until then a read may observe a state older than one
observed before the restart.

The recovery signal is the cluster commit, established only by the current leader replicating to
a quorum — it is never recovered from local storage. After a restart this node perceives it only
once the current leader has re-established the commit by committing its own-term (blank) entry to
a quorum. That re-established commit is at least every previously committed — hence previously
applied — log id, so once the state machine applies up to it, no read can be reverted. This is
the same `read_log_id` condition that linearizable reads rely on; see [`docs::protocol::read`]
and [`docs::protocol::commit`].

To avoid serving a reverted read, either:

- Persist `committed` (recommended): on restart the recovered `committed` is the true
  pre-restart value, and the state machine is re-applied up to it on startup.
- Wait for recovery before serving reads: [`Raft::wait_for_recovery`] blocks until the
  re-established cluster commit has been applied.

```ignore
let raft = Raft::new(id, config, network, log_store, sm).await?;
raft.wait_for_recovery(Some(Duration::from_secs(5))).await?;
```

### Why

The reason of adding a persisted `committed` is to just make things clear.
Without persisting `committed`:

- The `RaftMetrics.last_applied` will fall back upon startup, which may confuse a monitoring service.
- The application may depend on the state in the state machine, e.g., by saving node information in the state machine. State falling back may cause a node to try to contact a removed node.

Although these issues can all be addressed, it introduces more work to make things strictly correct.
A state machine that won't revert to a former state is easier to use:)

### Overhead

The overhead introduced by calling `save_committed()` should be minimal: in average, it will be called for every `max_payload_entries` log entries.
Meanwhile, I do not quite worry about the penalty, unless there is a measurable overhead.

[`RaftLogStorage`]: `crate::storage::RaftLogStorage`
[`RaftLogStorage::save_committed`]: `crate::storage::RaftLogStorage::save_committed`
[`docs::protocol::read`]: `crate::docs::protocol::read`
[`docs::protocol::commit`]: `crate::docs::protocol::commit`
[`Raft::wait_for_recovery`]: `crate::Raft::wait_for_recovery`
