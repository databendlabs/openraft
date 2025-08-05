# `ReplicationSessionId`

In OpenRaft, data often needs to be replicated from one node to multiple target
nodes—a process known as **replication**. To guarantee the correctness and
consistency of the replication process, the `ReplicationSessionId` is introduced
as an identifier that distinguishes individual replication sessions. This
ensures that replication states are correctly handled during leader changes or
membership modifications.

When a Leader starts replicating log entries to a set of target nodes, it
initiates a replication session that is uniquely identified by the
`ReplicationSessionId`. The session comprises the following key elements:
- The identity of the Leader (`leader_id`)
- The set of replication targets (`targets: BTreeSet<NodeId>`)

The `ReplicationSessionId` uniquely identifies a replication stream from Leader
to target nodes. When replication progresses (e.g., Node A receives log entry
10), the replication module sends an update message (`{target=A,
matched=log_id(10)}`) with the corresponding `ReplicationSessionId` to track
progress accurately.

The conditions that trigger the creation of a new `ReplicationSessionId` include:
- A change in the Leader’s identity.
- A change in the cluster’s membership.

This mechanism ensures that replication states remain isolated between sessions.

## Structure of `ReplicationSessionId`

The Rust implementation defines the structure as follows:

```rust,ignore
pub struct ReplicationSessionId {
    /// The Leader this replication belongs to.
    pub(crate) leader_vote: CommittedVote,

    /// The log id of the membership log this replication works for.
    pub(crate) membership_log_id: Option<LogId>,
}
```

The structure contains two core elements:

1. **`leader_vote`**
   This field identifies the Leader that owns this replication session. When a
   new Leader is elected, the previous replication session becomes invalid,
   preventing state mixing between different Leaders.

2. **`membership_log_id`**
   This field stores the ID of the membership log for this replication session.
   When membership changes via a log entry, a new replication session is created.
   The `membership_log_id` ensures old replication states are not reused with
   new membership configurations.

## Isolation Example

Consider a scenario with three membership logs:

1. `log_id=1`: members = {a, b, c}
2. `log_id=5`: members = {a, b}
3. `log_id=10`: members = {a, b, c}

The process occurs as follows:

- When `log_id=1` is appended, OpenRaft starts replicating to Node `c`. After
  log entry `log_id=1` has been replicated to Node `c`, an update message
  `{target=c, matched=log_id-1}` is enqueued (into a channel) for processing by the
  Raft core.

- When `log_id=5` is appended, due to the membership change, replication to Node
  `c` is terminated.

- When `log_id=10` is appended, a new replication session to Node `c` is
  initiated. At this point, Node `c` is considered a newly joined node without
  any logs.

Without proper session isolation, a delayed state update message from a previous
session (e.g., `{target=c, matched=log_id-1}`) could be mistakenly applied to a
new replication session. This would cause the Raft core to incorrectly believe
Node `c` has received and committed certain log entries. The `ReplicationSessionId`
prevents this by strictly isolating replication sessions when membership or Leader
changes occur.

## Consequences of Not Distinguishing Membership Configurations

If the replication session only distinguishes based on the Leader and not the
`membership_log_id`, the Leader at `log_id=10` may incorrectly believe Node `c`
has received `log_id=1`. While this does not cause data loss due to Raft's
membership change algorithm, it creates engineering problems: Node `c` will
return errors when receiving logs starting from `log_id=2` since it's missing
`log_id=1`. The Leader may misinterpret these errors as data corruption and
trigger protective actions like service shutdown.

### Why Data Loss Does Not Occur

Raft itself provides specific guarantees ensuring no committed data will be lost:

- Before proposing a membership configuration with `log_id=10` (denoted as
  `c10`), the previous membership configuration must have been committed.
- If the previous configuration were `c8` (associated with `log_id=8`), then
  `c8` would have been committed to a quorum within the cluster. This implies
  that the preceding `log_id=1` had already been accepted by a quorum.
- Raft’s joint configuration change mechanism ensures that there is an
  overlapping quorum between `c8` and `c10`. Consequently, any new Leader,
  whether elected under `c8` or `c10`, will have visibility of the already
  committed `log_id=1`.
- Based on these principles, committed data is safeguarded against loss.

### Potential Engineering Issues

Even though data loss is avoided, improper session isolation can still lead to
engineering challenges:

- If the Leader erroneously assumes that `log_id=1` has been replicated on Node
  `c`, it continues to send further log entries. Node `c`, however, will respond
  with errors due to the missing `log_id=1`.
- The Leader may then mistakenly conclude that Node `c` has experienced data
  loss, prompting the triggering of protective operations, which might, in turn,
  lead to service downtime.
