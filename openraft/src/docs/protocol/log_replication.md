# Replication by Append-Entries

Raft logs can be collectively considered as a **single value**:
An append-entry RPC sends all logs to a follower and replaces all logs on the follower.
This ensures that committed logs are always visible to the next leader.

However, in practice, it's unfeasible to send all the logs in a single RPC.
Therefore, the receiving end in the algorithm is modified as follows:
- Proceed only when `prev_log_id` matches the local log id at the same `index`.
- Save every log entry into the local store if:
    - the entry at the target index is empty.
    - the entry at the target index is the same as the input one.
      Otherwise, there's an **inconsistent** entry,
      and the follower must **delete** all entries starting from this one before storing the input one.

## Necessity to delete conflicting logs

In Raft, it is crucial to ensure that all nodes have a consistent state.
When conflicting logs are present, it can lead to data loss issues.
Because **that the next leader always chooses committed logs**:

-   When a leader is elected, it is essential that it has the most up-to-date log entries,
    i.e., the committed logs from previous leader.

-   The conflicting logs on a follower `A` may have a smaller log ID than the last log ID on the leader.
    Therefore,
    the next leader may choose another node `B` that has a **greater** log than node
    `A` but has a **smaller** log than the previous leader.
  
    This can lead to data loss.

By deleting conflicting logs, it ensures that the next leader will have the most up-to-date and committed logs.
The following diagram shows only the log terms, `*` indicates modified states.
In this example:
- (1) Initially, R3 and R5 have conflicting logs, `3-0, 3-1, 3-2` vs `2-0, 2-1, 2-2`;
- (2) If R1, the current leader in term 5, replicates logs to R3 but R3 does not delete conflicting logs `3-1, 3-2`, R1 believes `5-0` is committed;
- (3) When R5 becomes the leader (max last-log-id wins), it will override the **committed** log entry `5-0` on R3.

To prevent this, deleting the conflicting logs and replicating the leader's logs
is required to ensure consistency across all nodes.


In this scenario,
it is essential for R3 to delete the conflicting logs `3-1, 3-2`
when R1 replicates the logs, so that `R5` won't become a leader and the system
can maintain consistency across all nodes.

```text
(1)  R1 |
     R2 |
     R3 | 3  3  3
     R4 |
     R5 | 2  4  4
     -----0--1--2---> log index

(2)  R1 | 5*
     R2 | 5*
     R3 | 5* 3  3
     R4 |
     R5 | 2  4  4
     -----0--1--2---> log index

(3)  R1 | 5
     R2 | 5
     R3 | 2* 4* 4*
     R4 |
     R5 | 2* 4* 4*
     -----0--1--2---> log index
```

## Caveat: Deleting all entries after `prev_log_id` may result in loss of committed logs

One mistake is that if [`prev_log_id`] is found, **delete all entries after `prev_log_id`** then append logs.
Such a scenario can cause committed data loss because deleting and appending are not executed atomically:

```ignore
fn handle_append_entries(req) {
    if store.has(req.prev_log_id) {
        store.delete_logs(req.prev_log_id.index..)
        store.append_logs(req.entries)
    }
}
```

For instance, consider the following log entries and assume R1 is the leader:

```text
R1 | 1-1  1-2  1-3
R2 | 1-1  1-2
R3 |
-----1----2----3---> log index
```

When the following steps occur, the committed entry `1-2` is lost:

- R1 to R2: `append_entries(entries=["1-2", "1-3"], prev_log_id="1-1")`
- R2 deletes `1-2`
- R2 crashes
- R2 is elected as the leader and only sees `1-1`; the committed entry `1-2` is lost.

**The safe approach is to skip every entry present in the append-entries
message, then delete only the inconsistent entries**.
This can be done by iterating through each entry in the append-entries message
and comparing it with the corresponding entry in the local logs.
If the entries match, skip them; if there's a mismatch, delete the inconsistent
entries in the local logs and then append the new entries. This ensures that
committed logs are not lost during log replication.

[`prev_log_id`]: `crate::raft::AppendEntriesRequest::prev_log_id`

## Caveat: commit-index must not advance beyond the last known consistent log

The commit index must be updated only after append-entries and must point to a
log entry that is **consistent** with the **current** leader.
Otherwise, there's a chance that an uncommitted entry could be applied on a follower:

```text
R0 | 1-1  1-2  3-3
R1 | 1-1  1-2  2-3
R2 | 1-1  1-2  3-3
-----1----2----3----> log index
```

- R0 to R1 append_entries: `entries=[1-2], prev_log_id = 1-1, commit_index = 3`
- R1 accepts this append-entries request but is not aware that entry `2-3` is
  inconsistent with the leader. Then it updates `commit_index` to `3` and
  applies `2-3`.


## Algorithm to find the last matching log id on a Follower

When a Leader is established, it does not know which log entries are present on
Followers and Learners. Therefore, it must determine the last matching log ID on
each Follower before sending append-entries.

This process utilizes a binary search:

The Leader uses a span `[matching_log_id, first_conflict_index]` to search:
- The left bound is the last known matching log ID.
- The right bound is the index of the first known log that the Follower does not have.

This span is defined in `ProgressEntry`, which the Leader uses to track
replication progress for each Follower/Learner.

```rust,ignore
pub(crate) struct ProgressEntry<C> {
    // Left bound
    pub(crate) matching: Option<LogId<C::NodeId>>,
    // Right bound
    pub(crate) searching_end: u64,
}
```

### Algorithm steps:

- The Leader initializes the span as `[None, leader_last_log_index + 1]`.
  Because the Leader is assumed to have all committed logs. Any logs after
  `leader_last_log_index` are uncommitted and can be safely deleted.

- The Leader sends the log entry at the midpoint of `[ProgressEntry.matching.next_index(), ProgressEntry.searching_end]` to the Follower.

- If the log entry is accepted, update `ProgressEntry.matching` to the current log ID.

- If the log entry is rejected, update `ProgressEntry.searching_end` to the current log index.

- Repeat the process until `matching.next_index() == searching_end`.

By following these steps, the Leader can efficiently find the last matching log
ID on a Follower using binary search.

Notes:

- `ProgressEntry.matching.next_index()` refers to the index right after the last
  known matching log ID.


## LogId Appended Multiple Times

Consider a scenario where a specific `LogId` is truncated and appended more than once.

The following discussion is in Raft terminology, a `term` and `LogId` are represented as `(term, index)`.

```text
Ni: Node
i-j: log with term i and index j

N1 | 1-1 1-2
N2 | 1-1 1-2
N3 | 3-1
N4 | 4-1
N5 |
N6 |
N7 |
-------------------------> log index
```

Given the initial cluster state built with the following steps:

1. **N1 as Leader**:
    - N1 established itself as the leader with a quorum of `N1, N5, N6, N7`.
    - N1 appended logs `1-1` and `1-2`, replicating only to N2.
2. **N3 as Leader**:
    - N3 became the leader with a quorum of `N3, N5, N6, N7`.
    - N3 appended log `3-1` but failed to replicate any logs to other nodes.
3. **N4 as Leader**:
    - N4 became the leader with a quorum of `N4, N5, N6, N7`.
    - N4 appended log `4-1` but also failed to replicate any logs.

As a result, N1's log will be truncated and appended multiple times:

- **Initial State**:
    - N1's term and logs are `1; 1-1, 1-2`.
- **N3 as Leader at Term 5**:
    - N3 established itself as the leader with a quorum of `N3, N5, N6, N7`.
    - N3 truncated all logs on N1 and replicated log `3-1` to N1 before crashing.
    - N1's term and log become `5; 3-1`.
- **N2 as Leader at Term 6**:
    - N2 established itself as the leader with a quorum of `N2, N5, N6, N7`.
    - N2 truncated all logs on N1 and replicated logs `1-1, 1-2` to N1 before crashing.
    - N1's term and logs become `6; 1-1, 1-2`.
- **N4 as Leader at Term 7**:
    - N4 established itself as the leader with a quorum of `N4, N5, N6, N7`.
    - N4 truncated all logs on N1 and replicated log `4-1` to N1 before crashing.
    - N1's term and log become `7; 4-1`.

This scenario can repeat, where a log entry with the same `LogId` is truncated and appended multiple times if there are enough nodes in the cluster.

However, it's important to note that a specific `LogId` can only be truncated and appended by a leader with a higher term.

Therefore, the pointer for an IO operation must be represented as `term, LogId(term, index)`.
In Openraft, the `LogIOId` is `(CommittedLeaderId, LogId)`.


[binary search]: https://en.wikipedia.org/wiki/Binary_search_algorithm
