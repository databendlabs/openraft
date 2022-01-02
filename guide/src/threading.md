# Threads(tasks)

There are several threads, AKA tokio-tasks in this raft impl:

- RaftCore: all log and state machine operation is done in this thread.
  Thus there is no race condition

  - All raft state runs in this task, such as LeaderState, CandidateState etc.
  - All write to store is done in this task, i.e., write to store is serialized.

  Lifecycle:

  - RaftCore thread is spawnd when a raft node is created and keeps running until
    the raft node is dropped.


- Replication tasks:

  There is exactly one replication task spawned for every replication target,
  i.e., a follower or learner.

  A replication task replicates logs or snapshot to its target.
  A replication thread do not write logs or state machine, but only read from it.

  Lifecycle:

  - A replication task is spawned when RaftCore enters `LeaderState`, or a leader
    target is added by user.

  - A replication task is dropped when a follower of learner is removed by
    **change-membership** or when RaftCore quits `LeaderState`.

- Snapshot building task:

  When RaftCore receives a RaftMsg that requires a snapshot, which is sent by a
  **replication task**, RaftCore spawns a sub task to build a snapshot.

  Lifecycle:

  - It is spawned when a snapshot is requested,
    and is dropped at once when the snapshot is ready.

- User application runs in another task that spawns RaftCore task.


# Communication between tasks

All tasks communicate with channels:

```
User
|
| write;
| change_membership;
| ...
|
|                     new log to
|                     replicate;
`---------> RaftCore -------------+-> Replication -.
            ^  ^                  |                |
            |  |                  `-> Replication -+
            |  |                                   |
            |  `-----------------------------------'
            |      update replication state;
            |      need snapshot;
            |
            |
            | snapshot is ready;
            |
            Build-snapshot

```

- User to RaftCore: `Raft` sends `RaftMsg` though `Raft.tx_api` to `RaftCore`,
    along with a channel for `RaftCore` to send back response.

- RaftCore to Replication: `RaftCore` stores a channel for every repliation
    task.
    The messages sent to replication task includes:
    - a new log id to replicate,
    - and the index the leader has committed.

- Replication to RaftCore:

    - Replication task send the already replicated log id
        to RaftCore through another per-replication channel.

    - Replication task send a `NeedSnapshot` request through the same channel to
        ask RaftCore to build a snapshot if there is no log a follower/learner
        needs.

- Build-snapshot to RaftCore: RaftCore spawn a separate task to build a snapshot
    asynchronously. When finished, the spawned task send to RaftCore a message
    including the snapshot info.
