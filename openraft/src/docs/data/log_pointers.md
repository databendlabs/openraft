### Understanding Openraft Log Pointers

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
 
- `committed`: the last log entry that has been committed(granted by a quorum and at the leader's term) by the leader, inclusive.

- `last_log`:  the last log entry in the log, inclusive.

Within openraft, the log pointers mentioned above always hold that:
`purged` ≤ `snapshot` ≤ `applied` ≤ `committed` ≤ `last_log`.

The log indexes of a follower are always behind or equal the leader's `last_log`.
If a follower's `last_log` is lagging behind the leader's `purged`, it will be replicated using a snapshot.

```text
       .- delayed follower
       |
øøøøøøøøøøøøøøøLLLLLLLLLLLLLLLLLLLLLLLLLL
-------+------+------+-----+------+-----+-----------> log index
              |      |     |      |     |
              |      |     |      |     ` last_log
              |      |     |      ` committed
              |      |     ` applied
              |      ` snapshot
              ` purged
```

