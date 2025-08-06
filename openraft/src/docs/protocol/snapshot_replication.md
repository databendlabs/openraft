# Snapshot-Based Replication

Snapshot replication can be considered a special type of log replication: it
replicates all **committed** logs from index 0 to a specified index, similar to
the append-entry process, it removes conflicting logs, before installing a
snapshot:

- (1) If the snapshot's logs match the logs already stored on a
  Follower/Learner, no action is taken before installing snapshot.

- (2) If the snapshot's logs conflict with the local logs, **ALL** non-committed
  logs will be removed before installing snapshot. Additionally, the
  membership roll must be rolled back to a previous, non-conflicting state.

After truncating conflicting logs, the snapshot is installed.
The final step is to purge logs the snapshot contains. 

## 1. Snapshot-Log conditions

There are four cases when a snapshot is about to install on a Follower/Learner:

1) Snapshot before `last_purged` log index:

This won't happen, because the snapshot last log id is smaller than the
committed log id: `snapshot.last_log_id < last_purged <= committed_log_id`

```text
     snapshot ----.
                  v
-----------------------llllllllll--->
```


2) Snapshot last log id matches the local last log id:

No action needed before installing snapshot.

```text
     snapshot ----.
                  v
--------------llllllllll------------>
```


3) Snapshot last log id conflicts the local last log id:

Delete all non-committed logs then install snapshot.

```text
     snapshot ----.
                  v
--------------lllxxxxxxx------------>
```


4) Snapshot last log id is after local last log id:

Delete all non-committed logs then install snapshot.
Because any log before snapshot.last_log_id are committed (store on a quorum),
thus are safe to delete.

```text
     snapshot ----.
                  v
----lllllllllll--------------------->
```

To summarize the above four cases:
- If snapshot last log id matches: nothing to do.
- If snapshot last log id does not match (conflict or no such log): delete all
  non-committed logs.

### Rationale of Conflicting Logs Handling

If the local log is in conflict with [`snapshot_meta.last_log_id`], it must
delete all **non-committed** logs, not only the logs after
[`snapshot_meta.last_log_id`].

If not to delete all non-committed logs, the following situation may occur:
Although the node with conflicting logs won't become a leader at once,
but it could become a leader when it receives more logs and stores them after
[`snapshot_meta.last_log_id`].

Because the logs (which might be conflicting) before
[`snapshot_meta.last_log_id`] is not deleted, there's a chance that this node
becomes the leader and uses these logs for replication.

> Why a node with conflicting log won't become a leader:
>
> According to the Raft specification, for this node to become a leader, it must
> contain all committed logs. However, the log entry at `last_applied.index` is
> not committed; thus the node cannot become a leader.


### Safe to Remove All Non-Committed Logs

For this reason, Openraft truncates **ALL** non-committed logs and this is safe,
because: `snapshot_meta.last_log_id` is committed (or it cannot be in a
snapshot). If the local log ID conflicts with `snapshot_meta.last_log_id`, a
quorum must exist that contains `snapshot_meta.last_log_id`. Therefore, it is
**safe to remove all logs** on this node.


## 2. Install Snapshot

Installing snapshot includes two steps:
- Save snapshot to disk.
- Replace state machine with the content of snapshot.

## 3. Purge Logs

The final step is to purge logs up to [`snapshot_meta.last_log_id`].
This step is necessary because:

- 1) A local log that is `<=` [`snapshot_meta.last_log_id`] may conflict with the leader, and cannot be used anymore.

- 2) There may be a hole in the logs, if `snapshot_last_log_id > local_last_log_id`:

    ```text
        snapshot ----.
                     v
    ----lllllllllll--------------------->
    ```

If this node crashes after installing snapshot and before purging logs,
the log will be purged the next start-up, in [`get_initial_state()`].


[`get_initial_state()`]: `crate::storage::StorageHelper::get_initial_state`
[`snapshot_meta.last_log_id`]: `crate::storage::SnapshotMeta::last_log_id`
