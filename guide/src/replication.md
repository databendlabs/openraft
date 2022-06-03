# Replication

Appending entry is the only RPC to replicate logs from leader to followers or learners.
Installing a snapshot can be seen as a special form of **appending logs**.

## Append-entry

Raft logs can be together seen as a **single value**:
An append-entry RPC forwards all logs to a follower and replace all the logs on the follower.
This way it guarantees committed log can always been seen by next leader.

Although in practice, it is infeasible sending all the logs in one RPC.
Thus, the receiving end in the algorithm becomes:
- Proceed only when `prev_log_id` matches local log id at the same `index`.
- Save every log entry into local store if:
    - the entry at the target index is empty.
    - the entry at the target index is the same as the input one.
  Otherwise, there is an **inconsistent** entry,
  the follower must delete all entries since this one before storing the input one.

### Why need to delete

The following diagram shows only log term.

```text
R1 5
R2 5
R3 5 3 3
R4
R5 2 4 4
```

If log 5 is committed by R1, and log 3 is not removed, R5 in future could become a new leader and overrides log
5 on R3.

### Caveat: deleting all entries after `prev_log_id` get committed log lost

One of the mistake is to delete all entries after `prev_log_id` when a matching `prev_log_id` is found, e.g.:
```
fn handle_append_entries(req) {
    if store.has(req.prev_log_id) {
        store.delete_logs(req.prev_log_id.index..)
        store.append_logs(req.entries)
    }
}

```

This results in loss of committed entry, because deleting and appending are not atomically executed.

E.g., the log entries are as following and R1 now is the leader:

```text
R1 1,1  1,2  1,3
R2 1,1  1,2
R3
```

When the following steps take place, committed entry `{1,2}` is lost:

- R1 to R2: `append_entries(entries=[{1,2}, {1,3}], prev_log_id={1,1})`
- R2 deletes `{1,2}`
- R2 crash
- R2 elected as leader and only see `{1,1}`; the committed entry `{1,2}` is lost.

**The safe way is to skip every entry that present in append-entries message then delete only the
inconsistent entries**.


### Caveat: commit-index must not advance the last known consistent log

Because we can not just delete `log[prev_log_id.index..]`, (which results in loss of committed
entry), the commit index must be updated only after append-entries
and must point to a log entry that is consistent to the leader.
Or there would be chance applying an uncommitted entry on a follower:

```text
R0 1,1  1,2  3,3
R1 1,1  1,2  2,3
R2 1,1  1,2  3,3
```

- R0 to R1 append_entries: `entries=[{1,2}], prev_log_id = {1,1}, commit_index = 3`
- R1 accepted this append-entries request but was not aware of that entry `{2,3}` is inconsistent to leader.
  Then it will update `commit_index` to `3` and apply `{2,3}`


## Snapshot replication

Snapshot replication can be considered as a special form of log replication:
It replicates all **committed** logs since the index-0 upto some index.

Similar to append-entry:

- (1) If the logs contained in the snapshot matches logs that are stored on a
    Follower/Learner, nothing is done.

- (2) If the logs conflicts with the local logs, local conflicting logs will be
    deleted. And effective membership has to be reverted to some previous
    non-conflicting one.


### Necessity to delete conflicting logs

**The `(2)` mentioned above is not necessary to do to achieve correctness.
It is done only for clarity**.

If the `last_applied`(`snapshot_meta.last_log_id`) conflict with the local log at `last_applied.index`,
It does **NOT** need to delete the conflicting logs.

Because the node that has conflicting logs won't become a leader:
If this node can become a leader, according to raft spec, it has to contain all committed logs.
But the log entry at `last_applied.index` is not committed, thus it can never become a leader.

But deleting conflicting logs make the state cleaner. :)
This way method such as `get_initial_state()` does not need to deal with
conditions such as that `last_log_id` can be smaller than `last_applied`.
