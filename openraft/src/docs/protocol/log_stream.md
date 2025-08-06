# Ensuring Consecutive Raft Log Replication

To stream log entries to a remote node,
log entries are read in more than one local IO operations.
There are issues to address to provide consecutivity on the remote end.

In a stream log replication protocol, Raft logs are read through multiple local
IO operations.
To ensure the integrity of log replication to remote nodes, addressing potential
disruptions in the log entry sequence is crucial.


## Problem Description

In a Raft-based system, there is no guarantee that sequential read-log
operations, such as `read log[i]` and `read log[i+1]`, will yield consecutive
log entries. The reason for this is that Raft logs can be truncated, which
disrupts the continuity of the log sequence. Consider the following scenario:

- Node-1, as a leader, reads `log[4]`.
- Subsequently, Node-1 receives a higher term vote and demotes itself.
- In the interim, a new leader emerges and truncates the logs on Node-1.
- Node-1 regains leadership, appends new logs, and attempts to read `log[5]`.

Now, the read operation for `log[5]` becomes invalid because the log at index 6,
term 5 (`6-5`) is no longer a direct successor to the log at index 4, term 4
(`4-4`). This violates Raft's log continuity requirement for replication.

To illustrate:

```text
i-j: Raft log at term i and index j

Node-1: | vote(term=4)
        | 2-3  2-4
               ^
               '--- Read log[4]

Node-1: | vote(term=5) // Higher term vote received
        | 2-3  2-4

Node-1: | vote(term=5) // Logs are truncated, 4-4 is replicated
        | 2-3  4-4
               ^
               '--- Truncation and replication by another leader

Node-1: | vote(term=6) // Node-1 becomes leader again
        | 2-3  4-4  6-5 // Append noop log

Node-1: | vote(term=6)
        | 2-3  4-4  6-5
                    ^
                    '--- Attempt to read log[5]
```


## Solution

To ensure consecutive log entries for safe replication, it is necessary to
verify that the term has not changed between log reads. The following updated
operations are proposed:

1. `read vote` (returns `v1`)
2. `read log[i]`
3. `read log[i+1]`
4. `read vote` (returns `v2`)

If `v1` is equal to `v2`, we can confidently say that `log[i]` and `log[i+1]`
are consecutive and the replication is safe. Therefore, `ReplicationCore` must
check the term (vote) as part of its replication process to ensure log
consecutivity.

## Conclusion

The current release (up to 0.9) mitigates this issue by immediately halting communication
with `ReplicationCore` to prevent any new replication commands from being
processed.

However, future iterations of `ReplicationCore` may operate more proactively.
To maintain the integrity of replication, `ReplicationCore`
must ensure the consecutivity in the above method.
