use base2histogram::Histogram;
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

/// Percentile stats whose fields all differ: `samples`, then `first`, `first + 1`, ... for the
/// percentiles in order.
fn distinct_stats(samples: u64, first: u64) -> PercentileStats {
    PercentileStats {
        samples,
        p0_1: first,
        p1: first + 1,
        p5: first + 2,
        p10: first + 3,
        p50: first + 4,
        p90: first + 5,
        p99: first + 6,
        p99_9: first + 7,
    }
}

/// A display whose values all differ, so that a value shown under another label or column changes
/// the text.
///
/// The distributions have distinct sample counts and percentiles. Each counter group counts its
/// first and its last name and leaves the others at zero. The log stage percentiles come from the
/// histograms of the chart, which keep small values exactly; the persisted-to-committed stage has
/// no sample.
fn populated_display() -> RuntimeStatsDisplay<C> {
    let mut command_counts = vec![0; CommandName::COUNT];
    command_counts[0] = 999;
    command_counts[CommandName::COUNT - 1] = 1_000;

    let mut raft_msg_counts = vec![0; RaftMsgName::COUNT];
    raft_msg_counts[0] = 3;
    raft_msg_counts[RaftMsgName::COUNT - 1] = u64::MAX;

    let mut notification_counts = vec![0; NotificationName::COUNT];
    notification_counts[0] = 7;
    notification_counts[NotificationName::COUNT - 1] = 1_234_567;

    let mut hist = LogStageHistograms::new();
    hist.proposed_to_received.record_n(1, 4);
    hist.proposed_to_received.record_n(2, 1);
    hist.received_to_submitted.record_n(2, 3);
    hist.submitted_to_persisted.record_n(1, 1);
    hist.submitted_to_persisted.record_n(3, 2);
    hist.committed_to_applied.record_n(1, 2);
    hist.proposed_to_applied.record_n(4, 2);
    hist.proposed_to_applied.record_n(6, 3);

    RuntimeStatsDisplay {
        mode: DisplayMode::default(),
        core_busy_duration: Duration::from_micros(1_500),
        core_busy_histogram: distinct_stats(11, 100),
        apply_batch: distinct_stats(12, 200),
        append_batch: distinct_stats(13, 300),
        replicate_batch: distinct_stats(14, 400),
        raft_msg_per_run: distinct_stats(15, 500),
        write_batch: distinct_stats(16, 600),
        raft_msg_budget: distinct_stats(17, 700),
        notification_budget: distinct_stats(18, 800),
        raft_msg_usage_permille: distinct_stats(19, 900),
        notification_usage_permille: distinct_stats(20, 1000),
        command_counts,
        raft_msg_counts,
        notification_counts,
        log_stage_percentiles: hist.percentile_stats_array(),
        log_stage_histograms: hist,
        log_stages: LogStages::new(1, 0),
    }
}

fn make_display(
    apply_batch: PercentileStats,
    command_counts: Vec<u64>,
    log_stage: [(&'static str, PercentileStats); 6],
) -> RuntimeStatsDisplay<C> {
    RuntimeStatsDisplay {
        mode: DisplayMode::default(),
        core_busy_duration: Duration::ZERO,
        core_busy_histogram: EMPTY_STATS,
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
    let s = d.compact().to_string();

    // Every distribution is shown; zero counts and stages without samples are not.
    let want = [
        "RuntimeStats { core_busy_duration: 0ns",
        "core_busy_histogram(us): [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "apply_batch: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "append_batch: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "replicate_batch: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "raft_msg_per_run: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "write_batch: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "raft_msg_budget: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "notification_budget: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "raft_msg_usage_permille: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "notification_usage_permille: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "commands: {}",
        "raft_msgs: {}",
        "notifications: {}",
        "log_stages(us): {} }",
    ]
    .join(", ");
    assert_eq!(want, s);
}

#[cfg(feature = "runtime-stats")]
#[test]
fn test_compact_populated() {
    let s = populated_display().compact().to_string();

    let want = [
        "RuntimeStats { core_busy_duration: 1.5ms",
        "core_busy_histogram(us): [samples: 11, P0.1: 100, P1: 101, P5: 102, P10: 103, P50: 104, P90: 105, P99: 106, P99.9: 107]",
        "apply_batch: [samples: 12, P0.1: 200, P1: 201, P5: 202, P10: 203, P50: 204, P90: 205, P99: 206, P99.9: 207]",
        "append_batch: [samples: 13, P0.1: 300, P1: 301, P5: 302, P10: 303, P50: 304, P90: 305, P99: 306, P99.9: 307]",
        "replicate_batch: [samples: 14, P0.1: 400, P1: 401, P5: 402, P10: 403, P50: 404, P90: 405, P99: 406, P99.9: 407]",
        "raft_msg_per_run: [samples: 15, P0.1: 500, P1: 501, P5: 502, P10: 503, P50: 504, P90: 505, P99: 506, P99.9: 507]",
        "write_batch: [samples: 16, P0.1: 600, P1: 601, P5: 602, P10: 603, P50: 604, P90: 605, P99: 606, P99.9: 607]",
        "raft_msg_budget: [samples: 17, P0.1: 700, P1: 701, P5: 702, P10: 703, P50: 704, P90: 705, P99: 706, P99.9: 707]",
        "notification_budget: [samples: 18, P0.1: 800, P1: 801, P5: 802, P10: 803, P50: 804, P90: 805, P99: 806, P99.9: 807]",
        "raft_msg_usage_permille: [samples: 19, P0.1: 900, P1: 901, P5: 902, P10: 903, P50: 904, P90: 905, P99: 906, P99.9: 907]",
        "notification_usage_permille: [samples: 20, P0.1: 1000, P1: 1001, P5: 1002, P10: 1003, P50: 1004, P90: 1005, P99: 1006, P99.9: 1007]",
        "commands: {UpdateIOProgress: 999",
        "Respond: 1000}",
        "raft_msgs: {AppendEntries: 3",
        "GetRuntimeStats: 18446744073709551615}",
        "notifications: {Notify::VoteResponse: 7",
        "Notify::ReplicateCleared: 1234567}",
        "log_stages(us): {1:proposed→received: [samples: 5, P0.1: 1, P1: 1, P5: 1, P10: 1, P50: 1, P90: 2, P99: 2, P99.9: 2]",
        "2:received→submitted: [samples: 3, P0.1: 2, P1: 2, P5: 2, P10: 2, P50: 2, P90: 2, P99: 2, P99.9: 2]",
        "3:submitted→persisted: [samples: 3, P0.1: 1, P1: 1, P5: 1, P10: 1, P50: 3, P90: 3, P99: 3, P99.9: 3]",
        "5:committed→applied: [samples: 2, P0.1: 1, P1: 1, P5: 1, P10: 1, P50: 1, P90: 1, P99: 1, P99.9: 1]",
        "1~5:proposed→applied: [samples: 5, P0.1: 4, P1: 4, P5: 4, P10: 4, P50: 6, P90: 6, P99: 6, P99.9: 6]} }",
    ]
    .join(", ");
    assert_eq!(want, s);
}

#[test]
fn test_core_busy_histogram_distribution_display() {
    let mut histogram = Histogram::<()>::new();
    for micros in 1..=7 {
        histogram.record(micros);
    }
    let stats = histogram.percentile_stats();
    let expected = PercentileStats {
        samples: 7,
        p0_1: 1,
        p1: 1,
        p5: 1,
        p10: 1,
        p50: 4,
        p90: 7,
        p99: 7,
        p99_9: 7,
    };
    assert_eq!(expected, stats);

    let rendered = format!("{stats}");
    assert_eq!(
        "[samples: 7, P0.1: 1, P1: 1, P5: 1, P10: 1, P50: 4, P90: 7, P99: 7, P99.9: 7]",
        rendered,
    );

    let duration = Duration::from_micros(28);
    let duration_text = format!("{duration:?}");
    let duration_field = format!("core_busy_duration: {duration_text}");
    let histogram_field = format!("core_busy_histogram(us): {rendered}");

    let mut display = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
    display.core_busy_duration = duration;
    display.core_busy_histogram = stats;
    let compact = format!("{}", display.compact());
    let compact_head = format!("RuntimeStats {{ {duration_field}, {histogram_field}");
    assert!(compact.starts_with(&compact_head));

    let mut display = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
    display.core_busy_duration = duration;
    display.core_busy_histogram = stats;
    let multiline = format!("{}", display.multiline());
    let multiline_head = format!("RuntimeStats:\n  {duration_field}\n  {histogram_field}\n");
    assert!(multiline.starts_with(&multiline_head));

    #[cfg(feature = "runtime-stats")]
    {
        let mut display = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
        display.core_busy_duration = duration;
        display.core_busy_histogram = stats;
        let human = format!("{}", display.human_readable());
        let table = [
            "╭──────┬──────────┬──────┬────┬────┬─────┬─────┬─────┬─────┬───────┬─────────────────────────────────╮",
            "│      │ #Samples │ P0.1 │ P1 │ P5 │ P10 │ P50 │ P90 │ P99 │ P99.9 │                                 │",
            "├──────┼──────────┼──────┼────┼────┼─────┼─────┼─────┼─────┼───────┼─────────────────────────────────┤",
            "│ Busy │        7 │    1 │  1 │  1 │   1 │   4 │   7 │   7 │     7 │ Wall time of one core-loop turn │",
            "╰──────┴──────────┴──────┴────┴────┴─────┴─────┴─────┴─────┴───────┴─────────────────────────────────╯",
        ]
        .join("\n");
        let busy = format!("Core busy duration: {duration_text}\n\nCore loop busy (us):\n{table}\n");
        assert!(human.starts_with(&busy));
    }
}

#[test]
fn test_multiline_empty() {
    let d = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
    let s = d.multiline().to_string();

    // The sections of zero counts and of stages without samples stay empty, and no log stage
    // segment is shown.
    let want = [
        "RuntimeStats:",
        "  core_busy_duration: 0ns",
        "  core_busy_histogram(us): [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "  apply_batch: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "  append_batch: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "  replicate_batch: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "  raft_msg_per_run: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "  write_batch: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "  raft_msg_budget: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "  notification_budget: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "  raft_msg_usage_permille: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "  notification_usage_permille: [samples: 0, P0.1: 0, P1: 0, P5: 0, P10: 0, P50: 0, P90: 0, P99: 0, P99.9: 0]",
        "  commands:",
        "  raft_msgs:",
        "  notifications:",
        "  log_stages(us):",
        "",
    ]
    .join("\n");
    assert_eq!(want, s);
}

#[cfg(feature = "runtime-stats")]
#[test]
fn test_multiline_populated() {
    let s = populated_display().multiline().to_string();

    let want = [
        "RuntimeStats:",
        "  core_busy_duration: 1.5ms",
        "  core_busy_histogram(us): [samples: 11, P0.1: 100, P1: 101, P5: 102, P10: 103, P50: 104, P90: 105, P99: 106, P99.9: 107]",
        "  apply_batch: [samples: 12, P0.1: 200, P1: 201, P5: 202, P10: 203, P50: 204, P90: 205, P99: 206, P99.9: 207]",
        "  append_batch: [samples: 13, P0.1: 300, P1: 301, P5: 302, P10: 303, P50: 304, P90: 305, P99: 306, P99.9: 307]",
        "  replicate_batch: [samples: 14, P0.1: 400, P1: 401, P5: 402, P10: 403, P50: 404, P90: 405, P99: 406, P99.9: 407]",
        "  raft_msg_per_run: [samples: 15, P0.1: 500, P1: 501, P5: 502, P10: 503, P50: 504, P90: 505, P99: 506, P99.9: 507]",
        "  write_batch: [samples: 16, P0.1: 600, P1: 601, P5: 602, P10: 603, P50: 604, P90: 605, P99: 606, P99.9: 607]",
        "  raft_msg_budget: [samples: 17, P0.1: 700, P1: 701, P5: 702, P10: 703, P50: 704, P90: 705, P99: 706, P99.9: 707]",
        "  notification_budget: [samples: 18, P0.1: 800, P1: 801, P5: 802, P10: 803, P50: 804, P90: 805, P99: 806, P99.9: 807]",
        "  raft_msg_usage_permille: [samples: 19, P0.1: 900, P1: 901, P5: 902, P10: 903, P50: 904, P90: 905, P99: 906, P99.9: 907]",
        "  notification_usage_permille: [samples: 20, P0.1: 1000, P1: 1001, P5: 1002, P10: 1003, P50: 1004, P90: 1005, P99: 1006, P99.9: 1007]",
        "  commands:",
        "    UpdateIOProgress: 999",
        "    Respond: 1000",
        "  raft_msgs:",
        "    AppendEntries: 3",
        "    GetRuntimeStats: 18446744073709551615",
        "  notifications:",
        "    Notify::VoteResponse: 7",
        "    Notify::ReplicateCleared: 1234567",
        "  log_stages(us):",
        "       1:proposed→received: n=5 p50=1 p90=2 p99=2 p99.9=2",
        "      2:received→submitted: n=3 p50=2 p90=2 p99=2 p99.9=2",
        "     3:submitted→persisted: n=3 p50=3 p90=3 p99=3 p99.9=3",
        "       5:committed→applied: n=2 p50=1 p90=1 p99=1 p99.9=1",
        "      1~5:proposed→applied: n=5 p50=6 p90=6 p99=6 p99.9=6",
        "",
    ]
    .join("\n");
    assert_eq!(want, s);
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

    let d = d.multiline();
    let s = format!("{}", d);
    println!("{s}");

    // Each proposal is shown relative to the first one, so the text depends on the recorded
    // instants alone.
    let want = [
        "  log_stage_segments (T: proposal of the first segment):",
        "    [0,10): T+0.00ns +0.00ns; proposed +0.00ns (0.00ns), received +1.00ms (1.00ms), submitted +0.00ns (1.00ms), persisted +2.00ms (3.00ms), committed +1.00ms (4.00ms), applied +1.00ms (5.00ms)",
        "    [10,20): T+5.00ms +5.00ms; proposed +0.00ns (0.00ns), received +1.00ms (1.00ms), submitted +1.00ms (2.00ms), persisted +3.00ms (5.00ms), committed +2.00ms (7.00ms), applied +1.00ms (8.00ms)",
        "    [20,30): T+12.00ms +7.00ms; proposed +0.00ns (0.00ns), received +1.00ms (1.00ms), submitted +1.00ms (2.00ms), persisted +8.00ms (10.00ms), committed +3.00ms (13.00ms), applied +3.00ms (16.00ms)",
    ];
    let start = s.find(want[0]).expect("the segments section");
    let got = s[start..].lines().collect::<Vec<_>>();
    assert_eq!(want.to_vec(), got);

    assert_eq!(s, format!("{}", d), "formatting again gives the same text");
}

#[test]
fn test_mode_selection() {
    let compact = populated_display().compact().to_string();
    let multiline = populated_display().multiline().to_string();
    let human = populated_display().human_readable().to_string();

    // A display starts in the compact mode; `mode()` selects the same mode as each builder, and the
    // last selection wins.
    assert_eq!(compact, populated_display().to_string());
    assert_eq!(compact, populated_display().mode(DisplayMode::Compact).to_string());
    assert_eq!(multiline, populated_display().mode(DisplayMode::Multiline).to_string());
    assert_eq!(human, populated_display().mode(DisplayMode::HumanReadable).to_string());
    assert_eq!(compact, populated_display().human_readable().compact().to_string());
    assert_eq!(multiline, populated_display().compact().multiline().to_string());

    assert_ne!(compact, multiline);
    if cfg!(feature = "runtime-stats") {
        assert_ne!(multiline, human);
    } else {
        assert_eq!(multiline, human, "without tables, it falls back to the multiline mode");
    }
}

#[cfg(feature = "runtime-stats")]
#[test]
fn test_human_readable_empty() {
    let d = make_display(EMPTY_STATS, vec![0; CommandName::COUNT], empty_log_stages());
    let s = d.human_readable().to_string();

    // The distribution tables are shown with zeros; the count and log stage tables and the chart,
    // which would have no row, are not.
    let want = [
        "Core busy duration: 0ns",
        "",
        "Core loop busy (us):",
        "╭──────┬──────────┬──────┬────┬────┬─────┬─────┬─────┬─────┬───────┬─────────────────────────────────╮",
        "│      │ #Samples │ P0.1 │ P1 │ P5 │ P10 │ P50 │ P90 │ P99 │ P99.9 │                                 │",
        "├──────┼──────────┼──────┼────┼────┼─────┼─────┼─────┼─────┼───────┼─────────────────────────────────┤",
        "│ Busy │        0 │    0 │  0 │  0 │   0 │   0 │   0 │   0 │     0 │ Wall time of one core-loop turn │",
        "╰──────┴──────────┴──────┴────┴────┴─────┴─────┴─────┴─────┴───────┴─────────────────────────────────╯",
        "Batch Sizes:",
        "╭─────────────┬──────────┬──────┬────┬────┬─────┬─────┬─────┬─────┬───────┬────────────────────────────────────╮",
        "│             │ #Samples │ P0.1 │ P1 │ P5 │ P10 │ P50 │ P90 │ P99 │ P99.9 │                                    │",
        "├─────────────┼──────────┼──────┼────┼────┼─────┼─────┼─────┼─────┼───────┼────────────────────────────────────┤",
        "│ Apply       │        0 │    0 │  0 │  0 │   0 │   0 │   0 │   0 │     0 │ Entries per state machine apply    │",
        "│ Append      │        0 │    0 │  0 │  0 │   0 │   0 │   0 │   0 │     0 │ Entries per storage append         │",
        "│ Replicate   │        0 │    0 │  0 │  0 │   0 │   0 │   0 │   0 │     0 │ Entries per replication RPC        │",
        "│ RaftMsg/run │        0 │    0 │  0 │  0 │   0 │   0 │   0 │   0 │     0 │ RaftMsgs per run_engine_commands() │",
        "│ Write       │        0 │    0 │  0 │  0 │   0 │   0 │   0 │   0 │     0 │ Client writes merged per batch     │",
        "╰─────────────┴──────────┴──────┴────┴────┴─────┴─────┴─────┴─────┴───────┴────────────────────────────────────╯",
        "Budget & Utilization:",
        "╭───────────────┬──────────┬──────┬────┬────┬─────┬─────┬─────┬─────┬───────┬─────────────────────────────────────╮",
        "│               │ #Samples │ P0.1 │ P1 │ P5 │ P10 │ P50 │ P90 │ P99 │ P99.9 │                                     │",
        "├───────────────┼──────────┼──────┼────┼────┼─────┼─────┼─────┼─────┼───────┼─────────────────────────────────────┤",
        "│ RaftMsgBudget │        0 │    0 │  0 │  0 │   0 │   0 │   0 │   0 │     0 │ Max RaftMsgs allowed per loop       │",
        "│ NotifyBudget  │        0 │    0 │  0 │  0 │   0 │   0 │   0 │   0 │     0 │ Max Notifications allowed per loop  │",
        "│ RaftMsgUsage‰ │        0 │    0 │  0 │  0 │   0 │   0 │   0 │   0 │     0 │ RaftMsg budget utilization (‰)      │",
        "│ NotifyUsage‰  │        0 │    0 │  0 │  0 │   0 │   0 │   0 │   0 │     0 │ Notification budget utilization (‰) │",
        "╰───────────────┴──────────┴──────┴────┴────┴─────┴─────┴─────┴─────┴───────┴─────────────────────────────────────╯",
        "",
    ]
    .join("\n");
    assert_eq!(want, s);
}

#[cfg(feature = "runtime-stats")]
#[test]
fn test_human_readable_populated() {
    let s = populated_display().human_readable().to_string();

    // Counts are grouped by thousands, up to `u64::MAX`. The chart draws a row for each bucket that
    // holds a sample, with one bar segment and one count per stage; its summaries agree with the
    // log stage table, which shows only the stages with samples.
    let want = [
        "Core busy duration: 1.5ms",
        "",
        "Core loop busy (us):",
        "╭──────┬──────────┬──────┬─────┬─────┬─────┬─────┬─────┬─────┬───────┬─────────────────────────────────╮",
        "│      │ #Samples │ P0.1 │  P1 │  P5 │ P10 │ P50 │ P90 │ P99 │ P99.9 │                                 │",
        "├──────┼──────────┼──────┼─────┼─────┼─────┼─────┼─────┼─────┼───────┼─────────────────────────────────┤",
        "│ Busy │       11 │  100 │ 101 │ 102 │ 103 │ 104 │ 105 │ 106 │   107 │ Wall time of one core-loop turn │",
        "╰──────┴──────────┴──────┴─────┴─────┴─────┴─────┴─────┴─────┴───────┴─────────────────────────────────╯",
        "Batch Sizes:",
        "╭─────────────┬──────────┬──────┬─────┬─────┬─────┬─────┬─────┬─────┬───────┬────────────────────────────────────╮",
        "│             │ #Samples │ P0.1 │  P1 │  P5 │ P10 │ P50 │ P90 │ P99 │ P99.9 │                                    │",
        "├─────────────┼──────────┼──────┼─────┼─────┼─────┼─────┼─────┼─────┼───────┼────────────────────────────────────┤",
        "│ Apply       │       12 │  200 │ 201 │ 202 │ 203 │ 204 │ 205 │ 206 │   207 │ Entries per state machine apply    │",
        "│ Append      │       13 │  300 │ 301 │ 302 │ 303 │ 304 │ 305 │ 306 │   307 │ Entries per storage append         │",
        "│ Replicate   │       14 │  400 │ 401 │ 402 │ 403 │ 404 │ 405 │ 406 │   407 │ Entries per replication RPC        │",
        "│ RaftMsg/run │       15 │  500 │ 501 │ 502 │ 503 │ 504 │ 505 │ 506 │   507 │ RaftMsgs per run_engine_commands() │",
        "│ Write       │       16 │  600 │ 601 │ 602 │ 603 │ 604 │ 605 │ 606 │   607 │ Client writes merged per batch     │",
        "╰─────────────┴──────────┴──────┴─────┴─────┴─────┴─────┴─────┴─────┴───────┴────────────────────────────────────╯",
        "Budget & Utilization:",
        "╭───────────────┬──────────┬──────┬──────┬──────┬──────┬──────┬──────┬──────┬───────┬─────────────────────────────────────╮",
        "│               │ #Samples │ P0.1 │   P1 │   P5 │  P10 │  P50 │  P90 │  P99 │ P99.9 │                                     │",
        "├───────────────┼──────────┼──────┼──────┼──────┼──────┼──────┼──────┼──────┼───────┼─────────────────────────────────────┤",
        "│ RaftMsgBudget │       17 │  700 │  701 │  702 │  703 │  704 │  705 │  706 │   707 │ Max RaftMsgs allowed per loop       │",
        "│ NotifyBudget  │       18 │  800 │  801 │  802 │  803 │  804 │  805 │  806 │   807 │ Max Notifications allowed per loop  │",
        "│ RaftMsgUsage‰ │       19 │  900 │  901 │  902 │  903 │  904 │  905 │  906 │   907 │ RaftMsg budget utilization (‰)      │",
        "│ NotifyUsage‰  │       20 │ 1000 │ 1001 │ 1002 │ 1003 │ 1004 │ 1005 │ 1006 │  1007 │ Notification budget utilization (‰) │",
        "╰───────────────┴──────────┴──────┴──────┴──────┴──────┴──────┴──────┴──────┴───────┴─────────────────────────────────────╯",
        "Commands:",
        "╭──────────────────┬───────╮",
        "│ Command          │ Count │",
        "├──────────────────┼───────┤",
        "│ UpdateIOProgress │   999 │",
        "│ Respond          │ 1,000 │",
        "╰──────────────────┴───────╯",
        "Raft Messages:",
        "╭─────────────────┬────────────────────────────╮",
        "│ Message         │                      Count │",
        "├─────────────────┼────────────────────────────┤",
        "│ AppendEntries   │                          3 │",
        "│ GetRuntimeStats │ 18,446,744,073,709,551,615 │",
        "╰─────────────────┴────────────────────────────╯",
        "Notifications:",
        "╭──────────────────────────┬───────────╮",
        "│ Notification             │     Count │",
        "├──────────────────────────┼───────────┤",
        "│ Notify::VoteResponse     │         7 │",
        "│ Notify::ReplicateCleared │ 1,234,567 │",
        "╰──────────────────────────┴───────────╯",
        "Log Stage Latencies (us):",
        "╭───────────────────────┬──────────┬──────┬────┬────┬─────┬─────┬─────┬─────┬───────╮",
        "│                       │ #Samples │ P0.1 │ P1 │ P5 │ P10 │ P50 │ P90 │ P99 │ P99.9 │",
        "├───────────────────────┼──────────┼──────┼────┼────┼─────┼─────┼─────┼─────┼───────┤",
        "│ 1:proposed→received   │        5 │    1 │  1 │  1 │   1 │   1 │   2 │   2 │     2 │",
        "│ 2:received→submitted  │        3 │    2 │  2 │  2 │   2 │   2 │   2 │   2 │     2 │",
        "│ 3:submitted→persisted │        3 │    1 │  1 │  1 │   1 │   3 │   3 │   3 │     3 │",
        "│ 5:committed→applied   │        2 │    1 │  1 │  1 │   1 │   1 │   1 │   1 │     1 │",
        "│ 1~5:proposed→applied  │        5 │    4 │  4 │  4 │   4 │   6 │   6 │   6 │     6 │",
        "╰───────────────────────┴──────────┴──────┴────┴────┴─────┴─────┴─────┴─────┴───────╯",
        "Log Stage Latency Distribution (us):",
        " range | █ 1:proposed→received ▒ 2:received→submitted ░ 3:submitted→persisted ▓ 4:persisted→committed ▞ 5:committed→applied",
        "[1, 2) | ███████████████████████░░░░░░▞▞▞▞▞▞▞▞▞▞▞  4 + 0 + 1 + 0 + 2",
        "[2, 3) | ██████▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒  1 + 3 + 0 + 0 + 0",
        "[3, 4) | ░░░░░░░░░░░  0 + 0 + 2 + 0 + 0",
        "█     1:proposed→received  total: 5  P50: 1  P90: 2  P99: 2",
        "▒    2:received→submitted  total: 3  P50: 2  P90: 2  P99: 2",
        "░   3:submitted→persisted  total: 3  P50: 3  P90: 3  P99: 3",
        "▓   4:persisted→committed  total: 0  P50: 0  P90: 0  P99: 0",
        "▞     5:committed→applied  total: 2  P50: 1  P90: 1  P99: 1",
    ]
    .join("\n");
    assert_eq!(want, s);
}
