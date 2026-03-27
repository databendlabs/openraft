//! Lifecycle stages of a log entry, from proposal to apply completion.

/// A lifecycle stage of a log entry.
///
/// Each variant corresponds to a timestamp recorded as the entry
/// moves through the Raft pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
#[allow(dead_code)]
pub enum Stage {
    /// When the application called `client_write()`.
    Proposed = 0,

    /// When `RaftCore` dequeued the write request from the API channel.
    /// The gap from `Proposed` reveals channel queueing delay.
    Received = 1,

    /// When the entry was submitted to the Raft-Log storage for persistence.
    ///
    /// The gap from `Received` represents the engine command queue delay:
    /// after processing the client-write message, `Engine` generates commands
    /// (including the storage append command) into `Engine.output`.
    /// This delay is the time between when the command is generated and when
    /// it is actually submitted to the log storage implementation.
    Appended = 2,

    /// When the Raft-Log storage confirmed the entry is persisted
    /// (i.e., the append-entries callback returned).
    /// The gap from `Appended` reveals storage I/O latency.
    Persisted = 3,

    /// When this node locally marked the entry as committed
    /// (i.e., `RaftCore` received the commit index update from the Engine).
    ///
    /// This is the local commit timestamp, not the cluster-wide quorum time.
    /// The local committed index is always equal to or smaller than the
    /// cluster-wide committed index, because:
    /// - It takes time for the leader's commit decision to propagate to this node.
    /// - A log entry may already be committed by the cluster quorum but not yet present in this
    ///   node's local log storage.
    ///
    /// The gap from `Persisted` reveals replication + quorum wait latency.
    Committed = 4,

    /// When the state machine finished applying the entry and reported the result
    /// back to `RaftCore`.
    ///
    /// The gap from `Committed` includes both apply queueing delay and the actual
    /// state machine apply execution time.
    Applied = 5,
}

#[allow(dead_code)]
impl Stage {
    /// Total number of lifecycle stages.
    pub const COUNT: usize = 6;

    /// Returns the index of this stage for use as an array index.
    pub const fn index(self) -> usize {
        self as usize
    }
}
