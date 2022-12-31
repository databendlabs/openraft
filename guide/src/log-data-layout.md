# Log Data Layout

There are 5 significant log ids reflecting raft log state:

```
old                           new
øøøøøøLLLLLLLLLLLLLLLLLLLL
+----+----+----+----+----+--->
     |    |    |    |    |
     |    |    |    |    '--- last_log_id
     |    |    |    '-------- committed
     |    |    '------------- applied
     |    '------------------ snapshot_last_log_id
     '----------------------- last_purged_log_id
```

These log ids follow a strict order:
`last_purged_log_id` ≤ `snapshot_last_log_id` ≤ `applied` ≤ `committed` ≤ `last_log_id`

- `last_log_id`: is the last known log entry.

- `committed`: the last log entry that is committed, i.e., accepted and
    persisted by a quorum of voters, and will always be seen by all future
    leaders.

- `applied`: the committed log entry that is applied to the state-machine.
    In raft only committed log entry can be applied.

- `snapshot_last_log_id`: the greatest log id that is included in the snapshot.
    Openraft takes a snapshot of the state-machine periodically.
    Therefore, a snapshot is a compacted format of a continuous range of log entries.

- `last_purged_log_id`: the id of the last purged log entries.
    Log entries that are included in a snapshot will be purged.
    openraft purges logs from the start up to a configured position, when a fresh snapshot is built,
    `last_purged_log_id` is the greatest id that openraft does not store.
