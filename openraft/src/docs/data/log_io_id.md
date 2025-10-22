# LogIOId: Monotonic I/O Operation Identifier

## Why LogId Alone Is Insufficient

The `LogId` itself is **not monotonic** when tracking I/O operations. A specific `LogId` can be appended, truncated, and then appended again by different leaders.

For example, consider a follower that receives log entry `1-2` (term=1, index=2):
1. Leader-1 appends logs `[1-1, 1-2, 1-3]` to the follower
2. Leader-2 truncates `[1-2, 1-3]` and appends `[2-2]`
3. Leader-3 truncates `[2-2]` and appends `[1-2]` again

In this scenario, `LogId(1, 2)` is appended multiple times, making it unsuitable as a unique identifier for I/O operations.

See: [LogId Appended Multiple Times](crate::docs::protocol::replication::log_replication#logid-appended-multiple-times)

## LogIOId Structure

To ensure monotonic progress tracking, Openraft uses `LogIOId`:

```rust,ignore
pub(crate) struct LogIOId<C> {
    /// The id of the leader that performs the log io operation
    pub(crate) committed_vote: CommittedVote<C>,

    /// The last log id that has been flushed to storage
    pub(crate) log_id: Option<LogId<C>>,
}
```

The `CommittedVote` identifies which leader (local or remote) performed the I/O operation.

## Why Vote Makes It Monotonic

`LogIOId` is monotonically increasing because:

1. **Leader identity increases monotonically**: The `CommittedVote` (leader term) increases monotonically across the entire cluster. A new leader always has a higher term than previous leaders.

2. **Log entries are appended in order**: Within the same leader's term, log entries are proposed or replicated sequentially, ensuring that `LogId` increases monotonically.

Therefore, `(CommittedVote, LogId)` is strictly monotonic: when comparing two `LogIOId` values, either they have different votes (and the one with the higher vote is greater), or they have the same vote (and the one with the higher `LogId` is greater).

## Example Scenario

```text
N1 | 1-1  1-2
N2 | 1-1  1-2
N3 | 3-1
N4 | 4-1
```

If N1's logs are truncated and re-appended:
- N3 becomes leader at term 5: `LogIOId = (Vote(term=5), LogId(3,1))`
- N3 replicates to N1: N1's `LogIOId = (Vote(term=5), LogId(3,1))`
- N2 becomes leader at term 6: `LogIOId = (Vote(term=6), LogId(1,2))`
- N2 replicates to N1: N1's `LogIOId = (Vote(term=6), LogId(1,2))`

Even though `LogId(1,2)` appears again, the `LogIOId` is different and strictly greater:
- First append: `(Vote(term=1), LogId(1,2))`
- Second append: `(Vote(term=6), LogId(1,2))`

Since `Vote(term=6) > Vote(term=1)`, the second `LogIOId` is strictly greater, maintaining monotonicity.

## Local and Remote Leaders

The leader that performs the log I/O operation can be either:
- **Local leader**: When this node is the leader, it appends entries to its own log store
- **Remote leader**: When this node is a follower, it receives and appends entries replicated from the remote leader

In both cases, the `CommittedVote` identifies the leader performing the operation, ensuring that I/O operations remain monotonically ordered regardless of leadership changes.

## See Also

- [IOId](crate::docs::data::io_id) - I/O operation identifier for both vote and log operations
- [Log I/O Progress](crate::docs::data::log_io_progress) - Three-stage I/O progress tracking
- [LogId Appended Multiple Times](crate::docs::protocol::replication::log_replication#logid-appended-multiple-times)
