# Delete conflicting logs

- When appending logs to a follower/learner, conflicting logs **has** be removed.
- When installing snapshot(another form of appending logs), conflicting logs **should** be removed.

## Why

### 1. Keep it clean

The first reason is to keep logs clean:
to keep log ids all in ascending order.


### 2. Committed has to be chosen

The second reason is to **let the next leader always choose committed logs**.

If a leader commits logs that already are replicated to a quorum,
the next leader has to have these log.
The conflicting logs on a follower `A` may have smaller log id than the last log id on the leader.
Thus, the next leader may choose another node `B` that has higher log than node `A` but has smaller log than the previous leader.

### 3. Snapshot replication does not have to delete conflicting logs

See: [Deleting-conflicting-logs-when-installing-snapshot](https://datafuselabs.github.io/openraft/replication.html#necessity-to-delete-conflicting-logs)

Deleting conflicting logs when installing snapshot is only for clarity.