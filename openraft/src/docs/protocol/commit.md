# Cluster-Committed vs Local Committed in Raft

This document explains the distinction between cluster-committed and local
committed log entries in OpenRaft, and why they may differ due to out-of-order
RPC delivery.

## Definitions

- **Cluster-committed**: A log entry replicated to a quorum and visible to all
  future leaders (Raft guarantee)
- **Local committed**: A log entry safe to apply to the local state machine

These differ when a commit notification arrives before the append RPC that
writes the committed entry.

## Notation

```text
Ni:     Node i
Ti:     Term i
Li:     Leader established in term i
Ei-j:   Log entry at term i and index j
```

## The Problem: Out-of-Order RPC Delivery(Without Vote Sync First)

When a leader change occurs, inflight `AppendEntries` RPCs from multiple leaders
can arrive at a node in any order. A commit notification from a newer leader may
arrive before append RPCs that will truncate and re-append entries.

**Example timeline**:

```text
N1 | T1 E11
N2 |         T2 E21
N3 |    E11           T3   E11   E32
N4 | T1      T2       T3   E11   E32
N5 | T1      T2       T3   E11   E32
------+------+--+-----+----+-----+--------------------------> time
      t1     t2 t3    t4   t5    t6

Timeline:
t1: L1 replicates E11 to N1
t2: N2 becomes leader L2 (term T2), proposes E21 instead
t3: L2 sends Append(T2, E21) to N1 (inflight)
t4: N3 becomes leader L3 (term T3)
t5: L3 replicates E11 to quorum (N3,N4,N5), E11 is cluster-committed
    L3 sends Append(T3, E11) and Commit(T3, E32) to N1 (inflight)

t6: RPCs arrive at N1 in this order:
1. Commit(T3, E32) arrives → N1 learns E11 is cluster-committed
   - N1's log: [E11 from T1]
   - Problem: E11 from T1 ≠ E11 from T3 (different leaders)
   - Cannot apply yet

2. Append(T2, E21) arrives → truncates E11, appends E21
   - N1's log: [E21 from T2]
   - cluster_committed still points to E11 (inconsistent)

3. Append(T3, E11) arrives → truncates E21, re-appends E11
   - N1's log: [E11 from T3]
   - Now safe to apply
```

**The issue**: A node must distinguish "E11 from T1" from "E11 from T3" even
though they have the same index. Only when the local entry is from the same (or
newer) leader as the committing leader is it safe to apply.

## Solution: Vote Synchronization Protocol

**Rule**: Vote must be synchronized before committed log ID.

Whenever a committed log ID is replicated to a node, the node must first accept
the leader's vote:
- **Leader-to-follower**: Follower accepts leader's vote before accepting
  committed log ID
- **Follower-to-follower**: If a follower copies committed log ID from another
  follower, it must also first copy the vote

**Why**: Vote acts as logical timestamp in distributed consensus. Always record
the timestamp when an event happens, never allow reverting the timestamp, and
never allow events to happen before the current timestamp. By enforcing
vote-first synchronization, the node's accepted vote is always up-to-date before
receiving any commit notification, eliminating the need to track leader identity
with the committed log ID.

### What Happens Without Vote-First Protocol

```text
Scenario: N1 accepts commit from T3 without first accepting vote from T3

Initial state:
- N1 has E11 from T1 (accepted_vote = T1)
- L3 commits E11 in T3

Without vote-first protocol:
1. N1 receives Commit(T3, E11) and accepts cluster_committed = E11
   - N1's accepted_vote is still T1
2. N1 receives Append(T2, E21) from old leader
   - Would truncate E11 and append E21
   - But cluster_committed still points to E11
   - Inconsistent state: committed index points to non-existent entry

With vote-first protocol:
1. N1 receives Vote(T3) and accepts it first (accepted_vote = T3)
2. N1 receives Commit(T3, E11) and accepts cluster_committed = E11
   - Safe because accepted_vote is already T3
3. Any old Append(T2, E21) is automatically rejected (T2 < T3)
```

The vote-first protocol ensures the node's vote is always synchronized before
accepting any commit notification, so inflight RPCs from older leaders are
automatically rejected.

### Implementation

**Standard Raft**: Bundles vote (term) and commit index in `AppendEntries` RPC,
synchronizing them atomically.

**OpenRaft**: Enforces vote-first protocol - the leader's vote must be
synchronized before any commit notification is sent. `cluster_committed` stores
`LogIOId = (CommittedLeaderId, LogId)` for:
- Self-documentation (records which leader sent the commit)
- Update safety (forces callers to provide the leader's vote)
- Implementation flexibility (allows alternative synchronization approaches in
  theory)

With vote-first protocol, the vote in `cluster_committed` always equals
`log_progress.accepted()` vote.

## Implementation in OpenRaft

### Data Structures

```rust,ignore
pub(crate) struct IOState<C> {
    /// Tracks log I/O progress (vote + log entries)
    log_progress: IOProgress<IOId<C>>,

    /// Tracks cluster-committed with leader identity
    /// Stores leader's vote for self-documentation and update safety
    cluster_committed: MonotonicIncrease<LogIOId<C>>,
}
```

Where:
- `IOId<C>` = `Vote` or `LogIOId` (leader + log_id)
- `LogIOId<C>` = `(CommittedLeaderId, Option<LogId>)`

### Calculation Logic

```rust,ignore
pub(crate) fn calculate_local_committed(&mut self) -> Option<LogId<C>> {
    let cluster_committed = self.cluster_committed.value()?;
    let accepted = self.log_progress.accepted()?;

    let cluster_committed_vote = cluster_committed.to_committed_vote();
    let accepted_vote = accepted.to_committed_vote()?;

    // Vote-first protocol guarantees: accepted_vote == cluster_committed_vote
    // This check always passes but serves as a defensive assertion
    if accepted_vote >= cluster_committed_vote {
        std::cmp::min(
            accepted.last_log_id(),
            cluster_committed.last_log_id()
        )
    } else {
        None
    }
}
```

## Summary

**Standard Raft**: Bundles commit in `AppendEntries` RPC, bounded by `index_of_last_new_entry`
**OpenRaft**: Decouples commit notifications for performance
**Challenge**: Inflight RPCs from multiple leaders arrive in any order
**Solution**: Enforce vote-first synchronization protocol
**Protocol**: Vote must synchronize before committed log ID (vote as logical timestamp)
**Implementation**: `cluster_committed` stores `LogIOId` for self-documentation
and update safety; vote comparison check always passes but serves as defensive
assertion

## Related Documentation

- `IOState::cluster_committed` - Field tracking cluster-committed log ID with leader identity
- `IOState::calculate_local_committed()` - Method calculating safe local committed log ID
- [IO Ordering](crate::docs::protocol::io_ordering)
- [LogIOId](crate::docs::data::log_io_id)
