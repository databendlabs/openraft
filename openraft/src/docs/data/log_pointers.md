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
