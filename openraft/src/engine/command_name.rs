use openraft_macros::VariantName;
use openraft_macros::since;

/// Enum representing the name of each `sm::Command` variant.
#[since(version = "0.10.1", change = "marked #[non_exhaustive]")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(VariantName)]
#[variant_name(prefix = "SM::")]
#[non_exhaustive]
pub enum SMCommandName {
    /// Build a snapshot from the state machine's latest state.
    BuildSnapshot,
    /// Get the latest built snapshot.
    GetSnapshot,
    /// Install a full snapshot.
    InstallFullSnapshot,
    /// Apply log entries.
    Apply,
    /// Run a caller-supplied function on the state machine.
    ExternalFunc,
}

/// Enum representing the name of each `Command` variant.
///
/// This provides an efficient way to identify command types without
/// string comparisons, useful for logging, metrics, and debugging.
#[since(version = "0.10.2", change = "added `ClearReplicate` variant")]
#[since(version = "0.10.1", change = "marked #[non_exhaustive]")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(VariantName)]
#[non_exhaustive]
pub enum CommandName {
    /// Update the IO progress without submitting IO.
    UpdateIOProgress,
    /// Append a range of entries to the log.
    AppendEntries,
    /// Send the committed log id to other nodes.
    ReplicateCommitted,
    /// Send a heartbeat to all other nodes.
    BroadcastHeartbeat,
    /// Save the committed log id and apply the logs up to it.
    SaveCommittedAndApply,
    /// Replicate log entries to a target.
    Replicate,
    /// Ask the replication task of a target to confirm that it no longer reads the log for the
    /// plan the leader dropped.
    ClearReplicate,
    /// Replicate a snapshot to a target.
    ReplicateSnapshot,
    /// Send a leadership transfer message to all other nodes.
    BroadcastTransferLeader,
    /// Close the replication and heartbeat streams.
    CloseReplicationStreams,
    /// Fail the pending linearizable reads after losing leadership.
    FailPendingReads,
    /// Rebuild the replication streams after a membership change.
    RebuildReplicationStreams,
    /// Save the vote to storage.
    SaveVote,
    /// Send a vote request to all other members.
    SendVote,
    /// Send a Pre-Vote request to all other members.
    SendPreVote,
    /// Purge the logs up to an index, inclusive.
    PurgeLog,
    /// Delete the logs after a log id that conflict with the leader.
    TruncateLog,
    /// A command for the state machine worker, named by the inner value.
    StateMachine(SMCommandName),
    /// Send a result to a caller.
    Respond,
}

#[cfg(test)]
mod command_name_test;
