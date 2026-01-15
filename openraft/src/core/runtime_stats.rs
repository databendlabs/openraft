use std::fmt;

#[cfg(feature = "runtime-stats")]
use tabled::builder::Builder;
#[cfg(feature = "runtime-stats")]
use tabled::settings::Alignment;
#[cfg(feature = "runtime-stats")]
use tabled::settings::Style;
#[cfg(feature = "runtime-stats")]
use tabled::settings::object::Columns;

use crate::base::histogram::Histogram;
use crate::base::histogram::PercentileStats;
use crate::core::NotificationName;
use crate::core::raft_msg::RaftMsgName;
use crate::engine::CommandName;

/// Display mode for [`RuntimeStatsDisplay`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplayMode {
    /// Compact single-line format (default).
    #[default]
    Compact,
    /// Multiline format with one piece of information per line.
    Multiline,
    /// Human-readable format with better formatting and spacing.
    HumanReadable,
}

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

    /// Histogram tracking the distribution of RaftMsg counts processed in a batch.
    ///
    /// This tracks how many RaftMsg are processed together before calling
    /// `run_engine_commands()`, helping identify message batching efficiency.
    pub raft_msg_per_run: Histogram,

    /// Histogram tracking the distribution of client write entries merged per batch.
    ///
    /// This tracks how many client write entries are merged together in each
    /// `RaftMsg::ClientWrite` after batching, helping identify batching efficiency.
    pub write_batch: Histogram,

    /// Histogram tracking the budget for RaftMsg processing.
    ///
    /// This tracks the maximum number of RaftMsg allowed to process in each
    /// `process_raft_msg()` call, helping understand load balancing behavior.
    pub raft_msg_budget: Histogram,

    /// Histogram tracking the budget for Notification processing.
    ///
    /// This tracks the maximum number of Notifications allowed to process in each
    /// `process_notification()` call, helping understand load balancing behavior.
    pub notification_budget: Histogram,

    /// Histogram tracking RaftMsg budget utilization in permille (0-1000).
    ///
    /// This tracks `processed * 1000 / budget` for each `process_raft_msg()` call,
    /// where 1000 means 100% utilization. Helps identify if the budget is well-tuned.
    pub raft_msg_usage_permille: Histogram,

    /// Histogram tracking Notification budget utilization in permille (0-1000).
    ///
    /// This tracks `processed * 1000 / budget` for each `process_notification()` call,
    /// where 1000 means 100% utilization. Helps identify if the budget is well-tuned.
    pub notification_usage_permille: Histogram,

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

    /// Count of each Notification type received.
    ///
    /// This tracks how many times each Notification type has been received,
    /// useful for understanding internal message patterns and debugging.
    /// Indexed by `NotificationName::index()`.
    pub notification_counts: Vec<u64>,
}

impl Default for RuntimeStats {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeStats {
    pub(crate) fn new() -> Self {
        Self {
            apply_batch: Histogram::<()>::new(),
            append_batch: Histogram::<()>::new(),
            replicate_batch: Histogram::<()>::new(),
            raft_msg_per_run: Histogram::<()>::new(),
            write_batch: Histogram::<()>::new(),
            raft_msg_budget: Histogram::<()>::new(),
            notification_budget: Histogram::<()>::new(),
            raft_msg_usage_permille: Histogram::<()>::new(),
            notification_usage_permille: Histogram::<()>::new(),
            command_counts: vec![0; CommandName::COUNT],
            raft_msg_counts: vec![0; RaftMsgName::COUNT],
            notification_counts: vec![0; NotificationName::COUNT],
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

    /// Record the receipt of a Notification.
    pub fn record_notification(&mut self, name: NotificationName) {
        self.notification_counts[name.index()] += 1;
    }

    /// Returns a displayable representation of the runtime statistics.
    ///
    /// All values are precomputed when calling this method, so the returned
    /// `RuntimeStatsDisplay` can be cheaply formatted multiple times.
    ///
    /// Use builder methods like `.human_readable()` to change the display mode.
    #[allow(dead_code)]
    pub fn display(&self) -> RuntimeStatsDisplay {
        RuntimeStatsDisplay {
            mode: DisplayMode::default(),
            apply_batch: self.apply_batch.percentile_stats(),
            append_batch: self.append_batch.percentile_stats(),
            replicate_batch: self.replicate_batch.percentile_stats(),
            raft_msg_per_run: self.raft_msg_per_run.percentile_stats(),
            write_batch: self.write_batch.percentile_stats(),
            raft_msg_budget: self.raft_msg_budget.percentile_stats(),
            notification_budget: self.notification_budget.percentile_stats(),
            raft_msg_usage_permille: self.raft_msg_usage_permille.percentile_stats(),
            notification_usage_permille: self.notification_usage_permille.percentile_stats(),
            command_counts: self.command_counts.clone(),
            raft_msg_counts: self.raft_msg_counts.clone(),
            notification_counts: self.notification_counts.clone(),
        }
    }
}

/// Precomputed display data for [`RuntimeStats`].
///
/// All values are computed upfront so `Display::fmt()` is cheap.
/// Use [`DisplayMode`] to control the output format.
#[allow(dead_code)]
pub struct RuntimeStatsDisplay {
    mode: DisplayMode,
    apply_batch: PercentileStats,
    append_batch: PercentileStats,
    replicate_batch: PercentileStats,
    raft_msg_per_run: PercentileStats,
    write_batch: PercentileStats,
    raft_msg_budget: PercentileStats,
    notification_budget: PercentileStats,
    raft_msg_usage_permille: PercentileStats,
    notification_usage_permille: PercentileStats,
    command_counts: Vec<u64>,
    raft_msg_counts: Vec<u64>,
    notification_counts: Vec<u64>,
}

#[allow(dead_code)]
impl RuntimeStatsDisplay {
    /// Set the display mode.
    pub fn mode(mut self, mode: DisplayMode) -> Self {
        self.mode = mode;
        self
    }

    /// Use compact single-line format.
    pub fn compact(self) -> Self {
        self.mode(DisplayMode::Compact)
    }

    /// Use multiline format.
    pub fn multiline(self) -> Self {
        self.mode(DisplayMode::Multiline)
    }

    /// Use human-readable format.
    pub fn human_readable(self) -> Self {
        self.mode(DisplayMode::HumanReadable)
    }
}

impl fmt::Display for RuntimeStatsDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.mode {
            DisplayMode::Compact => self.fmt_compact(f),
            DisplayMode::Multiline => self.fmt_multiline(f),
            DisplayMode::HumanReadable => self.fmt_human_readable(f),
        }
    }
}

impl RuntimeStatsDisplay {
    fn fmt_compact(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RuntimeStats {{ apply_batch: {}, append_batch: {}, replicate_batch: {}, raft_msg_per_run: {}, write_batch: {}, raft_msg_budget: {}, notification_budget: {}, raft_msg_usage_permille: {}, notification_usage_permille: {}, commands: {{",
            self.apply_batch,
            self.append_batch,
            self.replicate_batch,
            self.raft_msg_per_run,
            self.write_batch,
            self.raft_msg_budget,
            self.notification_budget,
            self.raft_msg_usage_permille,
            self.notification_usage_permille
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

        write!(f, "}}, notifications: {{")?;

        let mut first = true;
        for (i, name) in NotificationName::ALL.iter().enumerate() {
            let count = self.notification_counts[i];
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

    fn fmt_multiline(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "RuntimeStats:")?;
        writeln!(f, "  apply_batch: {}", self.apply_batch)?;
        writeln!(f, "  append_batch: {}", self.append_batch)?;
        writeln!(f, "  replicate_batch: {}", self.replicate_batch)?;
        writeln!(f, "  raft_msg_per_run: {}", self.raft_msg_per_run)?;
        writeln!(f, "  write_batch: {}", self.write_batch)?;
        writeln!(f, "  raft_msg_budget: {}", self.raft_msg_budget)?;
        writeln!(f, "  notification_budget: {}", self.notification_budget)?;
        writeln!(f, "  raft_msg_usage_permille: {}", self.raft_msg_usage_permille)?;
        writeln!(f, "  notification_usage_permille: {}", self.notification_usage_permille)?;

        writeln!(f, "  commands:")?;
        for (i, name) in CommandName::ALL.iter().enumerate() {
            let count = self.command_counts[i];
            if count > 0 {
                writeln!(f, "    {}: {}", name, count)?;
            }
        }

        writeln!(f, "  raft_msgs:")?;
        for (i, name) in RaftMsgName::ALL.iter().enumerate() {
            let count = self.raft_msg_counts[i];
            if count > 0 {
                writeln!(f, "    {}: {}", name, count)?;
            }
        }

        writeln!(f, "  notifications:")?;
        for (i, name) in NotificationName::ALL.iter().enumerate() {
            let count = self.notification_counts[i];
            if count > 0 {
                writeln!(f, "    {}: {}", name, count)?;
            }
        }

        Ok(())
    }

    #[cfg(feature = "runtime-stats")]
    fn fmt_human_readable(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Batch sizes table
        writeln!(f, "Batch Sizes:")?;
        let mut builder = Builder::default();
        builder.push_record([
            "", "#Samples", "P0.1", "P1", "P5", "P10", "P50", "P90", "P99", "P99.9", "",
        ]);
        builder.push_record(Self::percentile_row(
            "Apply",
            &self.apply_batch,
            "Entries per state machine apply",
        ));
        builder.push_record(Self::percentile_row(
            "Append",
            &self.append_batch,
            "Entries per storage append",
        ));
        builder.push_record(Self::percentile_row(
            "Replicate",
            &self.replicate_batch,
            "Entries per replication RPC",
        ));
        builder.push_record(Self::percentile_row(
            "RaftMsg/run",
            &self.raft_msg_per_run,
            "RaftMsgs per run_engine_commands()",
        ));
        builder.push_record(Self::percentile_row(
            "Write",
            &self.write_batch,
            "Client writes merged per batch",
        ));
        let mut table = builder.build();
        table.with(Style::rounded());
        table.with(Alignment::right());
        table.modify(Columns::first(), Alignment::left());
        table.modify(Columns::last(), Alignment::left());
        writeln!(f, "{}", table)?;

        // Budget & utilization table
        writeln!(f, "Budget & Utilization:")?;
        let mut builder = Builder::default();
        builder.push_record([
            "", "#Samples", "P0.1", "P1", "P5", "P10", "P50", "P90", "P99", "P99.9", "",
        ]);
        builder.push_record(Self::percentile_row(
            "RaftMsgBudget",
            &self.raft_msg_budget,
            "Max RaftMsgs allowed per loop",
        ));
        builder.push_record(Self::percentile_row(
            "NotifyBudget",
            &self.notification_budget,
            "Max Notifications allowed per loop",
        ));
        builder.push_record(Self::percentile_row(
            "RaftMsgUsage‰",
            &self.raft_msg_usage_permille,
            "RaftMsg budget utilization (‰)",
        ));
        builder.push_record(Self::percentile_row(
            "NotifyUsage‰",
            &self.notification_usage_permille,
            "Notification budget utilization (‰)",
        ));
        let mut table = builder.build();
        table.with(Style::rounded());
        table.with(Alignment::right());
        table.modify(Columns::first(), Alignment::left());
        table.modify(Columns::last(), Alignment::left());
        writeln!(f, "{}", table)?;

        // Commands table with right-aligned counts
        let mut builder = Builder::default();
        builder.push_record(["Command", "Count"]);
        for (i, name) in CommandName::ALL.iter().enumerate() {
            let count = self.command_counts[i];
            if count > 0 {
                builder.push_record([name.to_string(), Self::format_count(count)]);
            }
        }
        if builder.count_records() > 1 {
            writeln!(f, "Commands:")?;
            let mut table = builder.build();
            table.with(Style::rounded());
            table.modify(Columns::last(), Alignment::right());
            writeln!(f, "{}", table)?;
        }

        // Raft messages table with right-aligned counts
        let mut builder = Builder::default();
        builder.push_record(["Message", "Count"]);
        for (i, name) in RaftMsgName::ALL.iter().enumerate() {
            let count = self.raft_msg_counts[i];
            if count > 0 {
                builder.push_record([name.to_string(), Self::format_count(count)]);
            }
        }
        if builder.count_records() > 1 {
            writeln!(f, "Raft Messages:")?;
            let mut table = builder.build();
            table.with(Style::rounded());
            table.modify(Columns::last(), Alignment::right());
            writeln!(f, "{}", table)?;
        }

        // Notifications table with right-aligned counts
        let mut builder = Builder::default();
        builder.push_record(["Notification", "Count"]);
        for (i, name) in NotificationName::ALL.iter().enumerate() {
            let count = self.notification_counts[i];
            if count > 0 {
                builder.push_record([name.to_string(), Self::format_count(count)]);
            }
        }
        if builder.count_records() > 1 {
            writeln!(f, "Notifications:")?;
            let mut table = builder.build();
            table.with(Style::rounded());
            table.modify(Columns::last(), Alignment::right());
            write!(f, "{}", table)?;
        }

        Ok(())
    }

    /// Create a row with percentile values for the batch sizes table.
    #[cfg(feature = "runtime-stats")]
    fn percentile_row(name: &str, stats: &PercentileStats, comment: &str) -> [String; 11] {
        [
            name.to_string(),
            Self::format_count(stats.samples),
            stats.p0_1.to_string(),
            stats.p1.to_string(),
            stats.p5.to_string(),
            stats.p10.to_string(),
            stats.p50.to_string(),
            stats.p90.to_string(),
            stats.p99.to_string(),
            stats.p99_9.to_string(),
            comment.to_string(),
        ]
    }

    /// Fallback when tabled is not available.
    #[cfg(not(feature = "runtime-stats"))]
    fn fmt_human_readable(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_multiline(f)
    }

    /// Format count with thousand separators for readability.
    #[cfg(feature = "runtime-stats")]
    fn format_count(n: u64) -> String {
        let s = n.to_string();
        let mut result = String::new();
        for (i, c) in s.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                result.push(',');
            }
            result.push(c);
        }
        result.chars().rev().collect()
    }
}
