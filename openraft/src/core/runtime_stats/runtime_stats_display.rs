use std::fmt;

#[cfg(feature = "runtime-stats")]
use tabled::builder::Builder;
#[cfg(feature = "runtime-stats")]
use tabled::settings::Alignment;
#[cfg(feature = "runtime-stats")]
use tabled::settings::Style;
#[cfg(feature = "runtime-stats")]
use tabled::settings::object::Columns;

use crate::core::NotificationName;
use crate::core::raft_msg::RaftMsgName;
#[cfg(doc)]
use crate::core::runtime_stats::RuntimeStats;
use crate::core::runtime_stats::display_mode::DisplayMode;
use crate::engine::CommandName;
use crate::raft_state::PercentileStats;

/// Precomputed display data for [`RuntimeStats`].
///
/// All values are computed upfront so `Display::fmt()` is cheap.
/// Use [`DisplayMode`] to control the output format.
#[allow(dead_code)]
pub struct RuntimeStatsDisplay {
    pub(crate) mode: DisplayMode,
    pub(crate) apply_batch: PercentileStats,
    pub(crate) append_batch: PercentileStats,
    pub(crate) replicate_batch: PercentileStats,
    pub(crate) raft_msg_per_run: PercentileStats,
    pub(crate) write_batch: PercentileStats,
    pub(crate) raft_msg_budget: PercentileStats,
    pub(crate) notification_budget: PercentileStats,
    pub(crate) raft_msg_usage_permille: PercentileStats,
    pub(crate) notification_usage_permille: PercentileStats,
    pub(crate) command_counts: Vec<u64>,
    pub(crate) raft_msg_counts: Vec<u64>,
    pub(crate) notification_counts: Vec<u64>,
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

    /// Format count with a thousand separators for readability.
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
