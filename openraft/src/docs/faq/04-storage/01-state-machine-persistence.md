### Does the state machine need to be persisted to disk?

**Question**: Should I persist the state machine to disk, or just rely on snapshots and log replay?

**Answer**: The state machine does not need to be persisted separately, since snapshots are periodically saved. On startup, rebuild the state machine from the latest snapshot.

Whether to re-apply raft logs after loading a snapshot depends on whether your application stores the committed log id using [`RaftLogStorage::save_committed()`][]:

- **If `save_committed()` is implemented**: Re-apply logs from the snapshot's last included log up to the saved committed log id on startup
- **If `save_committed()` is NOT implemented**: No log replay needed - the snapshot represents the committed state

This avoids the redundancy of persisting both the full state machine and its snapshot representation.

[`RaftLogStorage::save_committed()`]: `crate::storage::RaftLogStorage::save_committed`
