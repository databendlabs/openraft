# IOId: I/O Operation Identifier

`IOId` uniquely identifies I/O operations that make progress in the Raft log, supporting both vote persistence and log entry appends.

## Why Vote and Log Are Treated Together

In Openraft, `save_vote()` (persisting term) and `append()` (persisting log entries) I/O operations are **tightly coupled** and must not be reordered. This is why `IOId` encompasses both types of operations.

When a follower receives log entries from a leader with a higher term:
1. The follower must first persist the new term (vote)
2. Then persist the log entries

If these operations are reordered and the system crashes between them, committed data can be lost.

For a detailed explanation of why vote and log I/O cannot be reordered, see:
[I/O Ordering](crate::docs::protocol::io_ordering).

Because of this tight coupling, Openraft treats vote and log entries as a **unified whole** when tracking I/O progress. This is why:
- Both operations share the same `IOProgress` tracker
- Both contribute to the monotonic I/O sequence
- Both must respect strict ordering guarantees

## Structure

`IOId` is an enum with two variants:

```rust,ignore
pub(crate) enum IOId<C> {
    /// Saving a non-committed vote
    Vote(NonCommittedVote<C>),

    /// Saving log entries by a Leader
    Log(LogIOId<C>),
}
```

## What IOId Tracks

`IOId` tracks only I/O operations that **make progress** in the Raft log:

- **Included**: `save_vote()`, `append()` log entries
- **Excluded**: `purge()`, `truncate()`

Operations like `truncate()` just remove conflicting logs, and `purge()` removes already-committed logs. Neither makes forward progress, so they are not tracked by `IOId`.

## IOId vs LogIOId: When to Use Each

The key difference between `IOId` and `LogIOId`:
- **IOId** can represent **non-committed vote** I/O (saving term without log entries) OR log I/O
- **LogIOId** only represents **log I/O** with a committed vote (a leader appending entries)

Both can track vote information, but:
- `IOId::Vote(NonCommittedVote)` - saving term when voting for a candidate (no log entries)
- `IOId::Log(LogIOId)` - saving log entries, where `LogIOId` contains a `CommittedVote`
- `LogIOId` always contains a `CommittedVote` (never a non-committed vote)

### Use IOId When:

Need to track **both non-committed vote I/O and log I/O operations**:

- **Log storage progress** (`log_progress`): Uses `IOProgress<IOId<C>>` because storage I/O includes:
  - Non-committed vote I/O: When a node votes for a candidate, it saves the term (`IOId::Vote`)
  - Log I/O: When a node appends log entries from a leader (`IOId::Log`)

**Example**: `IOState.log_progress` uses `IOProgress<IOId<C>>` to track both vote-only I/O (when voting) and log I/O (when appending entries).

### Use LogIOId When:

Need to track **only log I/O** (which always has a committed vote):

- When tracking replication progress where only log entries matter
- When the context guarantees you're dealing with a committed leader's log operations

**Example**: In replication tracking, use `LogIOId` directly since you're only tracking log entry replication, not vote persistence.

### Use Plain LogId When:

Don't need to track vote information at all, only which logs:

- **Apply progress** (`apply_progress`): Uses `IOProgress<LogId<C>>` because it only tracks which committed logs are being applied to the state machine. Vote information is irrelevant.
- **Snapshot progress** (`snapshot`): Uses `IOProgress<LogId<C>>` because it only tracks which logs are included in snapshots. Vote information is irrelevant.

**Example**: `IOState.apply_progress` and `IOState.snapshot` use `IOProgress<LogId<C>>` because they only care about **which logs** are applied/snapshotted, not **which vote/leader** performed the I/O.

## Relationship Between IOId and LogIOId

`IOId` is a superset that contains `LogIOId`:

```rust,ignore
// Convert LogIOId to IOId
let io_id: IOId<C> = log_io_id.to_io_id();

// Extract LogIOId from IOId (if it's a Log variant)
match io_id {
    IOId::Log(log_io_id) => { /* use log_io_id */ }
    IOId::Vote(_) => { /* handle vote case */ }
}
```

## Comparison and Ordering

`IOId` implements `PartialOrd` by comparing votes first:

1. **Different votes**: The one with the higher vote is greater
2. **Same vote**: Compare the `last_log_id`
   - `IOId::Vote` has no log id (treated as `None`)
   - `IOId::Log` has an optional log id

This ensures monotonic ordering across both vote and log I/O operations:
- A vote I/O at term 5 is greater than any log I/O at term 4
- Among log I/Os at the same term, they're ordered by log id

## Example Usage

```rust,ignore
// Tracking both vote and log I/O
let mut progress = IOProgress::new_synchronized(
    None,
    "storage",
    "log",
    false
);

// Accept a vote I/O
let vote_io = IOId::new_vote_io(non_committed_vote);
progress.accept(vote_io);

// Accept a log I/O
let log_io = IOId::new_log_io(committed_vote, Some(log_id));
progress.accept(log_io);

// The progress now tracks both types of operations
```

## See Also

- [I/O Ordering](crate::docs::protocol::io_ordering) - Why vote and log I/O cannot be reordered
- [LogIOId](crate::docs::data::log_io_id) - Monotonic identifier for log I/O operations
- [Log I/O Progress](crate::docs::data::log_io_progress) - Three-stage I/O progress tracking
