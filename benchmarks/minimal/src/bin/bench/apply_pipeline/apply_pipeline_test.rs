use std::time::Duration;

use super::*;

fn fixture(limit: usize) -> Pipeline {
    let epoch = Arc::new(OnceLock::from(Instant::now()));
    let bounds = Bounds {
        start: Duration::from_nanos(10),
        end: Some(Duration::from_nanos(20)),
        deadline: Some(Duration::from_nanos(30)),
    };
    Pipeline::new(epoch, bounds, limit)
}

fn at(probe: &Pipeline, nanos: u64) -> Instant {
    *probe.epoch.get().unwrap() + Duration::from_nanos(nanos)
}

fn start_read(probe: &Pipeline, nanos: u64, range: Range<u64>) {
    let at = at(probe, nanos);
    probe.observe(PipelineEvent::ReadStarted { at, range: Some(range) });
}

fn finish_read(probe: &Pipeline, nanos: u64, entries: u64) {
    let at = at(probe, nanos);
    probe.observe(PipelineEvent::ReadFinished {
        at,
        entries: Some((entries, entries * 100)),
        status: "completed",
    });
}

fn read(probe: &Pipeline, interval: Range<u64>, logs: Range<u64>) {
    let count = logs.end - logs.start;
    start_read(probe, interval.start, logs);
    finish_read(probe, interval.end, count);
}

fn start_apply(probe: &Pipeline, nanos: u64) {
    let at = at(probe, nanos);
    probe.observe(PipelineEvent::ApplyStarted(at));
}

fn finish_apply(probe: &Pipeline, nanos: u64, logs: Range<u64>) {
    let published = at(probe, nanos - 1);
    let entries = logs.end - logs.start;
    let index = logs.end - 1;
    probe.observe(PipelineEvent::Applied {
        at: published,
        index,
        entries,
    });
    let ended = at(probe, nanos);
    probe.observe(PipelineEvent::ApplyFinished {
        at: ended,
        range: Some(logs),
        entries,
        payload_bytes: entries * 100,
        contiguous: true,
    });
}

#[test]
fn prefetch_and_tail_extension_have_independent_window_populations() {
    let probe = fixture(2);
    read(&probe, 1..4, 0..1);
    start_apply(&probe, 5);
    read(&probe, 6..12, 1..2);
    finish_apply(&probe, 14, 0..1);
    read(&probe, 15..18, 2..4);
    start_apply(&probe, 19);
    finish_apply(&probe, 25, 1..4);
    let report = probe.report();
    assert_eq!(5, report["all_reads"]["time_in_window_ns"]);
    assert_eq!(3, report["all_reads"]["unoverlapped_time_in_window_ns"]);
    assert_eq!(1, report["reads_per_apply"]["samples"]);
    assert_eq!(1, report["traces_dropped"]);
    assert_eq!(
        300,
        report["buffers"]["observed_peak_source_unattributed_referenced_payload_bytes"]
    );
}

#[test]
fn an_unfinished_prefetch_overruns_the_completed_apply() {
    let probe = fixture(8);
    read(&probe, 1..4, 0..1);
    start_apply(&probe, 5);
    start_read(&probe, 6, 1..2);
    finish_apply(&probe, 10, 0..1);
    finish_read(&probe, 15, 1);
    start_apply(&probe, 16);
    let report = probe.report();
    assert_eq!(5, report["all_reads"]["unoverlapped_ns"]["p50_ns"]);
    assert_eq!(5, report["all_reads"]["time_in_window_ns"]);
    assert_eq!(0, report["all_reads"]["overlap_in_window_ns"]);
    assert_eq!(true, report["conservation"]["read_entry_assignments"]);
}

#[test]
fn a_read_finishing_in_drain_keeps_its_window_time() {
    let probe = fixture(0);
    read(&probe, 5..25, 0..2);
    let report = probe.report();
    assert_eq!(10, report["all_reads"]["time_in_window_ns"]);
    assert_eq!(0, report["all_reads"]["completed_calls"]);
    assert_eq!(Value::Null, report["all_reads"]["duration_ns"]);
    assert_eq!(2, report["source_reads_unattributed"]["entries"]);
    assert_eq!(true, report["conservation"]["read_entry_assignments"]);
}

#[test]
fn incorrect_ranges_and_entry_counts_disable_accounting_validation() {
    let probe = fixture(0);
    start_read(&probe, 11, 0..2);
    finish_read(&probe, 12, 1);
    start_apply(&probe, 13);
    finish_apply(&probe, 15, 0..2);
    let report = probe.report();
    let expected = serde_json::json!({"read_calls":true,"apply_calls":true,"read_entry_assignments":false,
        "range_and_count_validation":false,"known_read_entry_counts":true,"known_read_ranges":true,
        "paired_events":true,"single_read_and_apply":true,"metadata_complete":true,"source_clocks_monotonic":true});
    assert_eq!(expected, report["conservation"]);
}

#[test]
fn cancellation_and_pending_work_remain_explicit() {
    let probe = fixture(0);
    read(&probe, 1..4, 0..1);
    start_apply(&probe, 5);
    let ended = at(&probe, 11);
    probe.observe(PipelineEvent::ApplyFinished {
        at: ended,
        range: Some(0..1),
        entries: 1,
        payload_bytes: 100,
        contiguous: true,
    });
    start_read(&probe, 12, 1..2);
    let report = probe.report();
    assert_eq!(1, report["inflight"]["read_calls"]);
    assert_eq!(1, report["all_observed_phase_counters"]["apply_ended_without_success"]);
    assert_eq!(
        1,
        report["all_observed_phase_counters"]["entries_assigned_to_unsuccessful_applies"]
    );
    assert_eq!(true, report["conservation"]["read_entry_assignments"]);
    assert_eq!(true, report["conservation"]["read_calls"]);
}

#[test]
fn one_read_can_overlap_two_applies_and_belong_to_the_following_batch() {
    let probe = fixture(16);
    read(&probe, 1..4, 0..1);
    start_apply(&probe, 5);
    read(&probe, 6..7, 1..2);
    start_read(&probe, 8, 2..3);
    finish_apply(&probe, 10, 0..1);
    start_apply(&probe, 11);
    finish_read(&probe, 15, 1);
    finish_apply(&probe, 18, 1..2);
    start_apply(&probe, 19);
    finish_apply(&probe, 25, 2..3);
    let report = probe.report();
    assert_eq!(1, report["all_reads"]["completed_calls"]);
    assert_eq!(1, report["all_reads"]["unoverlapped_ns"]["p50_ns"]);
    assert_eq!(4, report["all_reads"]["overlap_in_window_ns"]);
}

#[test]
fn failed_and_cancelled_read_counts_are_unknown_without_observed_corruption() {
    let probe = fixture(0);
    start_read(&probe, 11, 0..1);
    let failed = at(&probe, 12);
    probe.observe(PipelineEvent::ReadFinished {
        at: failed,
        entries: None,
        status: "failed",
    });
    start_read(&probe, 13, 1..2);
    let cancelled = at(&probe, 14);
    probe.observe(PipelineEvent::ReadFinished {
        at: cancelled,
        entries: None,
        status: "cancelled_or_panicked",
    });
    let report = probe.report();
    let expected = serde_json::json!({"read_calls":true,"apply_calls":true,"read_entry_assignments":true,
        "range_and_count_validation":true,"known_read_entry_counts":false,"known_read_ranges":true,
        "paired_events":true,"single_read_and_apply":true,"metadata_complete":true,"source_clocks_monotonic":true});
    assert_eq!(expected, report["conservation"]);
    assert_eq!(2, report["all_observed_phase_counters"]["unknown_read_entry_counts"]);
    assert_eq!(0, report["all_observed_phase_counters"]["range_or_count_mismatches"]);
}

#[test]
fn a_later_completed_read_remains_unattributed_when_only_the_prefix_applies() {
    let probe = fixture(8);
    read(&probe, 11..12, 0..1);
    read(&probe, 12..13, 1..2);
    start_apply(&probe, 14);
    finish_apply(&probe, 16, 0..1);
    let report = probe.report();
    let expected = serde_json::json!({"first":1,"end":2,"reads":1,"entries":1,"payload_bytes":100,"unknown_reads":0});
    assert_eq!(expected, report["source_reads_unattributed"]);
    assert_eq!(true, report["conservation"]["read_entry_assignments"]);
    start_apply(&probe, 18);
    finish_apply(&probe, 19, 1..2);
    let report = probe.report();
    assert_eq!(0, report["source_reads_unattributed"]["entries"]);
    assert_eq!(0, report["all_observed_phase_counters"]["range_or_count_mismatches"]);
}

#[test]
fn coalesced_source_chunks_belong_to_the_same_consumed_range() {
    let probe = fixture(8);
    read(&probe, 11..12, 0..1);
    read(&probe, 12..13, 1..3);
    start_apply(&probe, 14);
    finish_apply(&probe, 16, 0..3);
    let report = probe.report();
    assert_eq!(1, report["applies_with_multiple_reads"]);
    let expected =
        serde_json::json!({"first":null,"end":null,"reads":0,"entries":0,"payload_bytes":0,"unknown_reads":0});
    assert_eq!(expected, report["source_reads_unattributed"]);
    assert_eq!(true, report["conservation"]["read_entry_assignments"]);
}

#[test]
fn an_error_retains_the_unread_suffix_and_its_actual_payload() {
    let probe = fixture(8);
    read(&probe, 11..12, 0..4);
    read(&probe, 12..13, 4..5);
    start_apply(&probe, 14);
    let ended = at(&probe, 16);
    probe.observe(PipelineEvent::ApplyFinished {
        at: ended,
        range: Some(0..2),
        entries: 2,
        payload_bytes: 100,
        contiguous: true,
    });
    let report = probe.report();
    let expected = serde_json::json!({"first":2,"end":5,"reads":2,"entries":3,"payload_bytes":400,"unknown_reads":0});
    assert_eq!(expected, report["source_reads_unattributed"]);
    assert_eq!(
        2,
        report["all_observed_phase_counters"]["entries_assigned_to_unsuccessful_applies"]
    );
    assert_eq!(true, report["conservation"]["read_entry_assignments"]);
}

#[test]
fn callback_delivery_order_does_not_change_source_overlap() {
    let probe = fixture(8);
    read(&probe, 1..4, 0..1);
    start_read(&probe, 6, 1..2);
    start_apply(&probe, 5);
    let ready = std::sync::Barrier::new(2);
    let release = std::sync::Barrier::new(2);
    let ended = at(&probe, 12);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let event = PipelineEvent::ReadFinished {
                at: ended,
                entries: Some((1, 100)),
                status: "completed",
            };
            ready.wait();
            release.wait();
            probe.observe(event);
        });
        ready.wait();
        finish_apply(&probe, 14, 0..1);
        release.wait();
    });
    let report = probe.report();
    assert_eq!(2, report["all_reads"]["time_in_window_ns"]);
    assert_eq!(2, report["all_reads"]["overlap_in_window_ns"]);
    assert_eq!(0, report["all_reads"]["unoverlapped_time_in_window_ns"]);
    assert_eq!(true, report["overlap_coverage"]["complete"]);
    assert_eq!(1, report["overlap_coverage"]["retained_apply_spans"]);
}

#[test]
fn a_late_apply_end_defers_overlap_until_its_source_watermark() {
    let probe = fixture(8);
    read(&probe, 1..4, 0..1);
    start_apply(&probe, 5);
    read(&probe, 6..12, 1..2);
    let pending = probe.report();
    assert_eq!(false, pending["overlap_coverage"]["complete"]);
    assert_eq!(Value::Null, pending["all_reads"]["unoverlapped_time_in_window_ns"]);
    finish_apply(&probe, 14, 0..1);
    let resolved = probe.report();
    assert_eq!(true, resolved["overlap_coverage"]["complete"]);
    assert_eq!(2, resolved["all_reads"]["overlap_in_window_ns"]);
    assert_eq!(0, resolved["all_reads"]["unoverlapped_time_in_window_ns"]);
}

#[test]
fn bounded_metadata_overflow_preserves_totals_and_marks_unknown_geometry() {
    let probe = fixture(0);
    probe.state.lock().unwrap().retention_limit = 1;
    read(&probe, 11..12, 0..1);
    read(&probe, 12..13, 1..2);
    start_apply(&probe, 14);
    finish_apply(&probe, 16, 0..1);
    let report = probe.report();
    assert_eq!(2, report["all_observed_phase_counters"]["read_entries"]);
    assert_eq!(1, report["all_observed_phase_counters"]["applied_entries"]);
    assert_eq!(2, report["all_reads"]["completed_calls"]);
    assert_eq!(Value::Null, report["source_reads_unattributed"]);
    assert_eq!(
        Value::Null,
        report["buffers"]["observed_peak_source_unattributed_entries"]
    );
    assert_eq!(
        Value::Null,
        report["buffers"]["observed_peak_source_unattributed_referenced_payload_bytes"]
    );
    assert_eq!(1, report["all_observed_phase_counters"]["pending_metadata_overflow"]);
    assert_eq!(1, report["all_observed_phase_counters"]["overlap_metadata_overflow"]);
    assert_eq!(false, report["attribution_complete"]);
    assert_eq!(false, report["overlap_coverage"]["complete"]);
    assert_eq!(Value::Null, report["all_reads"]["unoverlapped_ns"]);
}

#[test]
fn a_reordered_stream_is_not_matched_by_endpoints_and_count() {
    let probe = fixture(0);
    read(&probe, 11..12, 0..4);
    start_apply(&probe, 14);
    let ended = at(&probe, 16);
    probe.observe(PipelineEvent::ApplyFinished {
        at: ended,
        range: Some(0..4),
        entries: 4,
        payload_bytes: 400,
        contiguous: false,
    });
    let report = probe.report();
    assert_eq!(4, report["source_reads_unattributed"]["entries"]);
    assert_eq!(1, report["all_observed_phase_counters"]["range_or_count_mismatches"]);
    assert_eq!(false, report["conservation"]["read_entry_assignments"]);
}

#[test]
fn incomplete_attribution_does_not_turn_a_later_range_gap_into_corruption() {
    let probe = fixture(0);
    probe.state.lock().unwrap().retention_limit = 1;
    read(&probe, 11..12, 0..1);
    read(&probe, 12..13, 1..2);
    start_apply(&probe, 14);
    finish_apply(&probe, 16, 0..2);
    read(&probe, 17..18, 10..11);
    let report = probe.report();
    assert_eq!(false, report["attribution_complete"]);
    assert_eq!(Value::Null, report["source_reads_unattributed"]);
    assert_eq!(true, report["conservation"]["range_and_count_validation"]);
    assert_eq!(0, report["all_observed_phase_counters"]["range_or_count_mismatches"]);
    start_read(&probe, 19, 20..22);
    finish_read(&probe, 20, 1);
    let malformed = probe.report();
    assert_eq!(1, malformed["all_observed_phase_counters"]["range_or_count_mismatches"]);
}
