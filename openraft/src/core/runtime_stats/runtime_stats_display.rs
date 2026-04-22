use std::fmt;

use base2histogram::PercentileStats;
#[cfg(feature = "runtime-stats")]
use tabled::builder::Builder;
#[cfg(feature = "runtime-stats")]
use tabled::settings::Alignment;
#[cfg(feature = "runtime-stats")]
use tabled::settings::Style;
#[cfg(feature = "runtime-stats")]
use tabled::settings::object::Columns;

use crate::RaftTypeConfig;
use crate::core::NotificationName;
use crate::core::raft_msg::RaftMsgName;
use crate::core::runtime_stats::display_mode::DisplayMode;
use crate::core::runtime_stats::log_stage::LogStageHistograms;
use crate::core::runtime_stats::log_stage::LogStages;
use crate::engine::CommandName;
use crate::type_config::alias::InstantOf;

/// Precomputed display data for [`RuntimeStats`].
///
/// All values are computed upfront so `Display::fmt()` is cheap.
/// Use [`DisplayMode`] to control the output format.
#[allow(dead_code)]
pub struct RuntimeStatsDisplay<C>
where C: RaftTypeConfig
{
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
    pub(crate) log_stage_percentiles: [(&'static str, PercentileStats); 6],
    pub(crate) log_stage_histograms: LogStageHistograms,
    pub(crate) log_stages: LogStages<InstantOf<C>>,
}

#[allow(dead_code)]
impl<C> RuntimeStatsDisplay<C>
where C: RaftTypeConfig
{
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

impl<C> fmt::Display for RuntimeStatsDisplay<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.mode {
            DisplayMode::Compact => self.fmt_compact(f),
            DisplayMode::Multiline => self.fmt_multiline(f),
            DisplayMode::HumanReadable => self.fmt_human_readable(f),
        }
    }
}

impl<C> RuntimeStatsDisplay<C>
where C: RaftTypeConfig
{
    #[allow(dead_code)]
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

        write!(f, "}}, log_stages(us): {{")?;

        let mut first = true;
        for (name, stats) in &self.log_stage_percentiles {
            if stats.samples > 0 {
                if !first {
                    write!(f, ", ")?;
                }
                write!(f, "{}: {}", name, stats)?;
                first = false;
            }
        }

        write!(f, "}} }}")
    }

    #[allow(dead_code)]
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

        writeln!(f, "  log_stages(us):")?;
        for (name, stats) in &self.log_stage_percentiles {
            if stats.samples > 0 {
                writeln!(
                    f,
                    "    {:>22}: n={} p50={} p90={} p99={} p99.9={}",
                    name, stats.samples, stats.p50, stats.p90, stats.p99, stats.p99_9,
                )?;
            }
        }

        let mut lines = self.log_stages.display_lines().peekable();
        if lines.peek().is_some() {
            writeln!(f, "  log_stage_segments:")?;
            for line in lines {
                writeln!(f, "    {}", line)?;
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
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
            writeln!(f, "{}", table)?;
        }

        // Log stage latencies table (microseconds)
        let mut builder = Builder::default();
        builder.push_record(["", "#Samples", "P0.1", "P1", "P5", "P10", "P50", "P90", "P99", "P99.9"]);
        for (name, stats) in &self.log_stage_percentiles {
            if stats.samples > 0 {
                builder.push_record([
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
                ]);
            }
        }
        if builder.count_records() > 1 {
            writeln!(f, "Log Stage Latencies (us):")?;
            let mut table = builder.build();
            table.with(Style::rounded());
            table.with(Alignment::right());
            table.modify(Columns::first(), Alignment::left());
            writeln!(f, "{}", table)?;

            let chart = self.log_stage_histograms.ascii_chart();
            writeln!(f, "Log Stage Latency Distribution (us):")?;
            write!(f, "{}", chart.detailed())?;
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
    #[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use base2histogram::PercentileStats;

    use super::*;
    use crate::engine::testing::UTConfig;

    type C = UTConfig<()>;

    const EMPTY_STATS: PercentileStats = PercentileStats {
        samples: 0,
        p0_1: 0,
        p1: 0,
        p5: 0,
        p10: 0,
        p50: 0,
        p90: 0,
        p99: 0,
        p99_9: 0,
    };

    fn sample_stats(samples: u64, p50: u64, p99: u64) -> PercentileStats {
        PercentileStats {
            samples,
            p0_1: 1,
            p1: 2,
            p5: 5,
            p10: 10,
            p50,
            p90: p99 - 1,
            p99,
            p99_9: p99 + 1,
        }
    }

    fn make_display(
        apply_batch: PercentileStats,
        command_counts: Vec<u64>,
        log_stage: [(&'static str, PercentileStats); 6],
    ) -> RuntimeStatsDisplay<C> {
        RuntimeStatsDisplay {
            mode: DisplayMode::default(),
            apply_batch,
            append_batch: EMPTY_STATS,
            replicate_batch: EMPTY_STATS,
            raft_msg_per_run: EMPTY_STATS,
            write_batch: EMPTY_STATS,
            raft_msg_budget: EMPTY_STATS,
            notification_budget: EMPTY_STATS,
            raft_msg_usage_permille: EMPTY_STATS,
            notification_usage_permille: EMPTY_STATS,
            command_counts,
            raft_msg_counts: vec![0; RaftMsgName::COUNT],
            notification_counts: vec![0; NotificationName::COUNT],
            log_stage_percentiles: log_stage,
            log_stage_histograms: LogStageHistograms::new(),
            log_stages: LogStages::new(1, 0),
        }
    }

    fn empty_log_stages() -> [(&'static str, PercentileStats); 6] {
        [
            ("1:proposed→received", EMPTY_STATS),
            ("2:received→submitted", EMPTY_STATS),
            ("3:submitted→persisted", EMPTY_STATS),
            ("4:persisted→committed", EMPTY_STATS),
            ("5:committed→applied", EMPTY_STATS),
            ("1~5:proposed→applied", EMPTY_STATS),
        ]
    }

    #[test]
    fn test_compact_empty() {
        let d = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
        let s = format!("{}", d.compact());

        assert!(s.starts_with("RuntimeStats {"));
        assert!(s.ends_with("} }"));
        assert!(s.contains("commands: {}"));
        assert!(s.contains("log_stages(us): {}"));
    }

    #[test]
    fn test_compact_with_data() {
        let mut cmds = vec![0u64; CommandName::COUNT];
        cmds[0] = 42;

        let mut stages = empty_log_stages();
        stages[0].1 = sample_stats(10, 100, 500);

        let d = make_display(sample_stats(5, 50, 200), cmds, stages);
        let s = format!("{}", d.compact());
        println!("{s}");

        assert!(s.contains("apply_batch: [samples: 5"));
        assert!(s.contains("1:proposed→received: [samples: 10"));
    }

    #[test]
    fn test_multiline_empty() {
        let d = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
        let s = format!("{}", d.multiline());

        assert!(s.contains("RuntimeStats:"));
        assert!(s.contains("apply_batch:"));
        assert!(s.contains("log_stages(us):"));
        // No log stage entries when all empty
        assert!(!s.contains("proposed→received"));
    }

    #[test]
    fn test_multiline_with_log_stages() {
        let mut stages = empty_log_stages();
        stages[0].1 = sample_stats(100, 50, 200);
        stages[5].1 = sample_stats(100, 120, 800);

        let d = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], stages);
        let s = format!("{}", d.multiline());

        println!("{s}");

        assert!(s.contains("1:proposed→received"));
        assert!(s.contains("n=100"));
        assert!(s.contains("p50=50"));
        assert!(s.contains("p99=200"));
        assert!(s.contains("1~5:proposed→applied"));
        assert!(s.contains("p50=120"));
        // Stages with 0 samples are omitted
        assert!(!s.contains("2:received→submitted"));
    }

    #[test]
    fn test_multiline_with_log_stage_segments() {
        use std::time::Duration;

        use crate::type_config::TypeConfigExt;

        // Build 3 batches of log entries with realistic stage timestamps:
        //   batch 1: log [0,10), proposed at t=0ms
        //   batch 2: log [10,20), proposed at t=5ms
        //   batch 3: log [20,30), proposed at t=12ms
        let base = C::now();
        let t = |ms: u64| -> InstantOf<C> { base + Duration::from_millis(ms) };

        let mut log_stages = LogStages::<InstantOf<C>>::new(10, 0);

        // Batch 1: fast path — small queue delay, quick persist/commit/apply
        log_stages.proposed(10, t(0));
        log_stages.received(10, t(1));
        log_stages.submitted(10, t(1));
        log_stages.persisted(10, t(3));
        log_stages.committed(10, t(4));
        log_stages.applied(10, t(5));

        // Batch 2: moderate — slightly slower persist and replication
        log_stages.proposed(20, t(5));
        log_stages.received(20, t(6));
        log_stages.submitted(20, t(7));
        log_stages.persisted(20, t(10));
        log_stages.committed(20, t(12));
        log_stages.applied(20, t(13));

        // Batch 3: slow — storage stall causes higher latency
        log_stages.proposed(30, t(12));
        log_stages.received(30, t(13));
        log_stages.submitted(30, t(14));
        log_stages.persisted(30, t(22));
        log_stages.committed(30, t(25));
        log_stages.applied(30, t(28));

        let mut d = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
        d.log_stages = log_stages;

        let s = format!("{}", d.multiline());
        println!("{s}");

        assert!(s.contains("log_stage_segments:"));
        // 3 segments
        assert!(s.contains("[0,10):"));
        assert!(s.contains("[10,20):"));
        assert!(s.contains("[20,30):"));
        // Stage names and durations present
        assert!(s.contains("proposed +"));
        assert!(s.contains("persisted +"));
        assert!(s.contains("applied +"));
    }

    #[test]
    fn test_multiline_no_segments_when_empty() {
        let d = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
        let s = format!("{}", d.multiline());

        assert!(!s.contains("log_stage_segments:"));
    }

    #[test]
    fn test_multiline_with_counts() {
        let mut cmds = vec![0u64; CommandName::COUNT];
        cmds[0] = 7;

        let d = make_display(EMPTY_STATS, cmds, empty_log_stages());
        let s = format!("{}", d.multiline());

        assert!(s.contains("commands:"));
        assert!(s.contains(": 7"));
    }

    #[cfg(feature = "runtime-stats")]
    #[test]
    fn test_mode_builder() {
        let d = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
        let compact = format!("{}", d.compact());

        let d = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
        let multiline = format!("{}", d.multiline());

        let d = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
        let human = format!("{}", d.human_readable());

        assert!(compact.starts_with("RuntimeStats {"));
        assert!(multiline.starts_with("RuntimeStats:"));
        assert!(human.contains("Batch Sizes:"));
        assert_ne!(compact, multiline);
        assert_ne!(multiline, human);
    }

    #[cfg(feature = "runtime-stats")]
    #[test]
    fn test_human_readable_empty() {
        let d = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
        let s = format!("{}", d.human_readable());

        assert!(s.contains("Batch Sizes:"));
        assert!(s.contains("Budget & Utilization:"));
        // No command/message/notification/log-stage tables when all counts are zero
        assert!(!s.contains("Commands:"));
        assert!(!s.contains("Raft Messages:"));
        assert!(!s.contains("Notifications:"));
        assert!(!s.contains("Log Stage Latencies"));
    }

    #[cfg(feature = "runtime-stats")]
    #[test]
    fn test_human_readable_with_data() {
        let mut cmds = vec![0u64; CommandName::COUNT];
        cmds[0] = 1_234;

        let mut msgs = vec![0u64; RaftMsgName::COUNT];
        msgs[0] = 56;

        let mut stages = empty_log_stages();
        stages[0].1 = sample_stats(100, 50, 200);
        stages[5].1 = sample_stats(100, 120, 800);

        // Build histograms where each stage follows a log-normal distribution
        // (Gaussian on the log scale) with a different center.
        // 7 geometrically-spaced bucket values; each stage peaks at a
        // different position and tapers symmetrically on the log axis.
        //
        // values(us):            8   20   50  120  300  700 1500
        // 1:proposed→received   20   12    5    2    1    1    1  (peak@8)
        // 2:received→submitted   8   20   12    5    2    1    1  (peak@20)
        // 5:committed→applied    1    2    8   20   12    5    1  (peak@120)
        // 4:persisted→committed  1    1    2    8   20   12    5  (peak@300)
        // 3:submitted→persisted  1    1    1    2    8   20   12  (peak@700)
        let mut hist = LogStageHistograms::new();
        let values: [u64; 7] = [8, 20, 50, 120, 300, 700, 1500];
        let counts: [&[u64; 7]; 5] = [
            &[20, 12, 5, 2, 1, 1, 1],
            &[8, 20, 12, 5, 2, 1, 1],
            &[1, 1, 1, 2, 8, 20, 12],
            &[1, 1, 2, 8, 20, 12, 5],
            &[1, 2, 8, 20, 12, 5, 1],
        ];
        let fields = [
            &mut hist.proposed_to_received,
            &mut hist.received_to_submitted,
            &mut hist.submitted_to_persisted,
            &mut hist.persisted_to_committed,
            &mut hist.committed_to_applied,
        ];
        for (field, stage_counts) in fields.into_iter().zip(counts) {
            for (&v, &n) in values.iter().zip(stage_counts) {
                field.record_n(v, n);
            }
        }
        for (i, &v) in values.iter().enumerate() {
            let total: u64 = counts.iter().map(|c| c[i]).sum();
            hist.proposed_to_applied.record_n(v, total);
        }

        let mut d = make_display(sample_stats(10, 30, 150), cmds, stages);
        d.raft_msg_counts = msgs;
        d.log_stage_histograms = hist;
        let s = format!("{}", d.human_readable());
        println!("{s}");

        // Batch sizes table
        assert!(s.contains("Batch Sizes:"));
        assert!(s.contains("Apply"));

        // Counts tables
        assert!(s.contains("Commands:"));
        assert!(s.contains("1,234"));
        assert!(s.contains("Raft Messages:"));

        // Log stage percentile table
        assert!(s.contains("Log Stage Latencies (us):"));
        assert!(s.contains("1:proposed→received"));
        assert!(s.contains("1~5:proposed→applied"));
        // ASCII chart
        assert!(s.contains("Log Stage Latency Distribution (us):"));
        assert!(s.contains("1:proposed→received"));
        // Chart contains bar characters
        assert!(s.contains('█'));
    }
}
