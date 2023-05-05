# Threads (tasks)

There are several threads, also known as tokio-tasks, in this Raft implementation:

-   **`RaftCore`**:
    All user request handling and log IO are done in this thread.

    All writes to the [`RaftLogStorage`] are done in this task, meaning writes
    to the store are serialized.

    **Lifecycle**:
    - The RaftCore thread is spawned when a Raft node is created and keeps
      running until the Raft node is dropped.


-   **Replication tasks**:
    There is exactly one replication task spawned for every target node
    (i.e., a follower or learner).

    A replication task replicates logs or snapshots to its target. A replication
    thread does not write logs or state machines but only reads from them.

    **Lifecycle**:
      - A replication task is spawned when `RaftCore` enters `LeaderState`.
      - A replication task is dropped when **change-membership** log take effects or when `RaftCore` quits `LeaderState`.


-   **StateMachine**:
    Handles all state-machine read and write operations, such as applying a committed log
    entry, or constructing a snapshot.

    **Lifecycle**:
    - Spawned when the `RaftCore` is initialized, and continues running until
      `RaftCore` is terminated.


-   **Snapshot building task**:
    Is a short term task for building a snapshot.

    **Lifecycle**:
      - The snapshot building task is spawned by [`StateMachine task`] when a
        snapshot is requested and is dropped once the snapshot is ready.


-   **User application**:
    runs in another task that spawns the RaftCore task and keeps a control
    handle [`Raft`].


# Communication between tasks

All tasks communicate with channels:

```text
User
|
| write;
| change_membership;
| ...
|
|                     new log to
|                     replicate;
`---------> RaftCore -------------+-> Replication -.
            ^|  ^                 |                |
            ||  |                 `-> Replication -+
            ||  |                           ^      |
            ||  `---------------------------|------'
            ||     update replication state;|
            ||     need snapshot;           |
            ||                              |
            ||                              |
            ||                              |
            ||  install_snapshot            |
            ||  apply_log                   |
            |'----------------> sm::Worker  |
            |                   |  |        |
            '-------------------'  |        |
                applied result     |        |
                                   |        |
                                   |        |
                                   |        |snapshot is ready;
                                   v        |
                                   Build-snapshot

```

- User to RaftCore: [`Raft`] sends `RaftMsg` through `Raft.tx_api` to `RaftCore`,
  `RaftMsg` contains a oneshot channel for `RaftCore` to send back a response.

- RaftCore to Replication: `RaftCore` maintains a channel for every replication
  task.
  The messages sent to the replication task include:
    - a new log ID to replicate,
    - and the index that the leader has committed.

- Replication to RaftCore:

    - Replication tasks send the already replicated log ID
      to RaftCore through another per-replication channel.

- RaftCore to `sm::Worker`: `RaftCore` forwards entries to apply to
  `sm::Worker`, and if snapshot-based replication is required, `RaftCore`
  sends a message to `sm::Worker`, which then spawns a task to build
  the snapshot.

- Build-snapshot to RaftCore: once the snapshot building is completed, the spawned
  task sends a message to `RaftCore` via `Notify` containing the snapshot information.

[`Raft`]:              `crate::raft::Raft`
[`ReplicationHandle`]: `crate::replication::ReplicationHandle`
[`ReplicationCore`]:   `crate::replication::ReplicationCore`
[`client_write`]:      `crate::raft::Raft::client_write`
[`RaftStorage`]:       `crate::storage::RaftStorage`
[`RaftLogStorage`]:    `crate::storage::RaftLogStorage`
[`RaftStateMachine`]:  `crate::storage::RaftStateMachine`
[`Adapter`]:           `crate::storage::Adapter`
[`RaftNetwork`]:       `crate::network::RaftNetwork`
[`append_entries`]:    `crate::network::Network::append_entries`
[`VoteRequest`]:       `crate::raft::VoteRequest`

[//]: # (private items)
[//]: # ([`RaftCore`]:          `crate::core::RaftCore`)
[//]: # ([`RaftMsg`]:           `crate::raft::RaftMsg`)
[//]: # ([`Notify`]:            `crate::core::notify::Notify`)
[//]: # ([`sm::Worker`]:        `crate::core::sm::Worker`)
