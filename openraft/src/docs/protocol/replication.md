# Replication Protocol

Openraft replicates log entries from the leader to followers/learners to maintain consistency across the cluster.

## Replication Methods

### Append-Entries Replication

The primary replication mechanism sends log entries incrementally:
- Leader sends new entries with `prev_log_id` to ensure consistency
- Follower verifies `prev_log_id` matches local state before accepting
- Conflicting entries are deleted to maintain consistency

See: [Replication by Append-Entries](`crate::docs::protocol::replication::log_replication`)

### Snapshot Replication

When a follower falls too far behind or log history becomes large:
- Leader sends a snapshot containing committed state up to a specific log index
- Removes conflicting uncommitted logs before installing snapshot
- More efficient than replaying all historical log entries

See: [Replication by Snapshot](`crate::docs::protocol::replication::snapshot_replication`)

## Key Concepts

**Log Consistency**: All nodes eventually converge to the same log history through conflict resolution

**Committed Logs**: Once replicated to a quorum, logs are committed and guaranteed to appear on future leaders

**Conflict Resolution**: Followers delete conflicting entries to ensure the next leader sees committed logs

## Replication Flow

1. Leader receives client write → appends to local log
2. Leader sends entries to followers via `append_entries` RPC
3. Followers verify consistency and store entries
4. Leader waits for quorum acknowledgment
5. Leader commits entries and notifies followers

For detailed protocol specifications, see the linked sub-pages above.
