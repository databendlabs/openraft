use std::fmt;

use crate::base::histogram::Histogram;
use crate::base::histogram::PercentileStats;
use crate::core::raft_msg::RaftMsgName;
use crate::engine::CommandName;

/// Runtime statistics for Raft operations.
///
/// This is a volatile structure that is not persisted. It accumulates
/// statistics from the time the Raft node starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStats {
    /// Histogram tracking the distribution of log entry counts in Apply commands.
    ///
    /// This tracks how many log entries are included in each apply command sent
    /// to the state machine, helping identify batch size patterns and I/O efficiency.
    pub apply_batch: Histogram,

    /// Histogram tracking the distribution of log entry counts when appending to storage.
    ///
    /// This tracks how many log entries are included in each AppendEntries command
    /// submitted to the storage layer, helping identify write batch patterns and storage I/O
    /// efficiency.
    pub append_batch: Histogram,

    /// Histogram tracking the distribution of log entry counts in replication RPCs.
    ///
    /// This tracks how many log entries are included in each AppendEntries RPC
    /// sent to followers during replication, helping identify replication batch patterns.
    pub replicate_batch: Histogram,

    /// Count of each command type executed.
    ///
    /// This tracks how many times each command type has been executed,
    /// useful for understanding workload patterns and debugging.
    /// Indexed by `CommandName::index()`.
    pub command_counts: Vec<u64>,

    /// Count of each RaftMsg type received.
    ///
    /// This tracks how many times each RaftMsg type has been received,
    /// useful for understanding API usage patterns and debugging.
    /// Indexed by `RaftMsgName::index()`.
    pub raft_msg_counts: Vec<u64>,
}

impl Default for RuntimeStats {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeStats {
    pub(crate) fn new() -> Self {
        Self {
            apply_batch: Histogram::new(),
            append_batch: Histogram::new(),
            replicate_batch: Histogram::new(),
            command_counts: vec![0; CommandName::COUNT],
            raft_msg_counts: vec![0; RaftMsgName::COUNT],
        }
    }

    /// Record the execution of a command.
    pub fn record_command(&mut self, name: CommandName) {
        self.command_counts[name.index()] += 1;
    }

    /// Record the receipt of a RaftMsg.
    pub fn record_raft_msg(&mut self, name: RaftMsgName) {
        self.raft_msg_counts[name.index()] += 1;
    }

    /// Returns a displayable representation of the runtime statistics.
    ///
    /// All values are precomputed when calling this method, so the returned
    /// `RuntimeStatsDisplay` can be cheaply formatted multiple times.
    #[allow(dead_code)]
    pub fn display(&self) -> RuntimeStatsDisplay {
        RuntimeStatsDisplay {
            apply_batch: self.apply_batch.percentile_stats(),
            append_batch: self.append_batch.percentile_stats(),
            replicate_batch: self.replicate_batch.percentile_stats(),
            command_counts: self.command_counts.clone(),
            raft_msg_counts: self.raft_msg_counts.clone(),
        }
    }
}

/// Precomputed display data for [`RuntimeStats`].
///
/// All values are computed upfront so `Display::fmt()` is cheap.
#[allow(dead_code)]
pub struct RuntimeStatsDisplay {
    apply_batch: PercentileStats,
    append_batch: PercentileStats,
    replicate_batch: PercentileStats,
    command_counts: Vec<u64>,
    raft_msg_counts: Vec<u64>,
}

impl fmt::Display for RuntimeStatsDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RuntimeStats {{ apply_batch: {}, append_batch: {}, replicate_batch: {}, commands: {{",
            self.apply_batch, self.append_batch, self.replicate_batch
        )?;

        let mut first = true;
        for (i, name) in CommandName::ALL.iter().enumerate() {
            let count = self.command_counts[i];
            if count > 0 {
                if !first {
                    write!(f, ", ")?;
                }
                write!(f, "{}: {}", name, count)?;
                first = false;
            }
        }

        write!(f, "}}, raft_msgs: {{")?;

        let mut first = true;
        for (i, name) in RaftMsgName::ALL.iter().enumerate() {
            let count = self.raft_msg_counts[i];
            if count > 0 {
                if !first {
                    write!(f, ", ")?;
                }
                write!(f, "{}: {}", name, count)?;
                first = false;
            }
        }

        write!(f, "}} }}")
    }
}
