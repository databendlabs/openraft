# Log I/O Progress Tracking

Openraft tracks the progress of log I/O operations through three distinct stages to maintain consistency and enable proper coordination between the Raft core and the storage layer.

Each I/O operation is uniquely identified by `IOId` or `LogIOId`. See:
- [IOId](crate::docs::data::io_id) - For tracking both vote and log I/O operations
- [LogIOId](crate::docs::data::log_io_id) - For tracking log I/O operations only

## IOProgress States

Each log I/O operation passes through three stages tracked by `IOProgress<LogIOId>`:

1. **Accepted**: The I/O operation has been accepted by the Raft core but not yet submitted to the storage layer
2. **Submitted**: The I/O operation has been submitted to the storage queue for processing
3. **Flushed**: The I/O operation has been successfully persisted to storage

The invariant maintained is: `flushed <= submitted <= accepted`

The window `(flushed, submitted]` represents I/O operations currently in progress.

## Why Track Multiple Stages?

Tracking these stages separately is critical for several reasons:

- **Non-blocking I/O**: The Raft core can accept new log entries without waiting for previous entries to be flushed to storage
- **Consistency guarantees**: The core knows which log entries are durable and can safely update the commit index only for flushed entries
- **Progress visibility**: The system can distinguish between entries accepted by the leader, entries in the I/O queue, and entries actually persisted

## Progress Tracking Example

Consider a leader appending log entries `1-1, 1-2, 1-3`:

```text
Time | Accepted       | Submitted      | Flushed
-----|----------------|----------------|----------------
t1   | (L1, 1-1)      | None           | None
t2   | (L1, 1-2)      | (L1, 1-1)      | None
t3   | (L1, 1-3)      | (L1, 1-2)      | (L1, 1-1)
t4   | (L1, 1-3)      | (L1, 1-3)      | (L1, 1-2)
t5   | (L1, 1-3)      | (L1, 1-3)      | (L1, 1-3)
```

At each point, the system knows:
- Which entries can be safely replicated (accepted)
- Which entries are being written to storage (submitted but not flushed)
- Which entries are durable and can contribute to commit index advancement (flushed)

## Implementation

The `IOProgress` struct maintains these three cursors and provides methods to update them:

- `accept(new_accepted)`: Update the accepted cursor when a new I/O operation is created
- `submit(new_submitted)`: Update the submitted cursor when an I/O operation is sent to storage
- `flush(new_flushed)`: Update the flushed cursor when storage confirms the operation is durable

In debug mode, `IOProgress` enforces monotonicity by default, panicking if updates arrive out of order. This can be disabled via the `allow_notification_reorder` flag for storage implementations that may report completions non-monotonically.
