//! Lifecycle stages of a log entry, from proposal to apply completion.

/// A lifecycle stage of a log entry.
///
/// Each variant corresponds to a timestamp recorded as the entry
/// moves through the Raft pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Stage {
    /// For client writes, just before sending the write message to `RaftCore`.
    /// If the core merges write messages, it keeps the first message's timestamp.
    /// For membership writes handled in the core, this is sampled just before `write_entries`.
    Proposed = 0,

    /// After `RaftCore` admits the write and `leader_append_entries` assigns log IDs,
    /// builds entries, and queues the storage append.
    ///
    /// For client writes, the gap from `Proposed` includes API channel queueing, batching,
    /// admission, and append preparation. It does not end at dequeue.
    Received = 1,

    /// Just before `RaftCore` calls `append` on the log store.
    ///
    /// The gap from `Received` includes the wait for the core to run the queued append command.
    Submitted = 2,

    /// When `RaftCore` handles the notification that local log I/O has completed,
    /// not when the storage callback runs.
    ///
    /// For a storage append, the gap from `Submitted` includes both storage I/O and the
    /// time it takes for the completion notification to reach and be processed by the core.
    Persisted = 3,

    /// When `RaftCore` starts `SaveCommittedAndApply` for the entry, before saving the
    /// committed log ID and dispatching the apply command.
    ///
    /// This observes local commit processing, not when a quorum committed the entry.
    Committed = 4,

    /// When `RaftCore` handles the state machine worker's apply result, not when the worker
    /// finishes applying the entry.
    ///
    /// The gap from `Committed` includes saving the committed log ID, apply queueing and
    /// execution, and delivery and handling of the apply result.
    Applied = 5,
}

impl Stage {
    /// Total number of lifecycle stages.
    pub const COUNT: usize = 6;

    /// Returns the index of this stage for use as an array index.
    pub const fn index(self) -> usize {
        self as usize
    }
}
