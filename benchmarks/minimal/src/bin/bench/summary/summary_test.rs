use std::time::Duration;

use super::Bounds;
use super::Outcome;
use super::RequestSample;
use super::Samples;

fn sample(scheduled_ns: u64, latency_ns: u64, outcome: Outcome) -> RequestSample {
    RequestSample {
        scheduled_ns,
        latency_ns,
        outcome,
        invoked_ns: None,
        invoked: false,
        node: None,
        retries: 0,
        phases: None,
        write_progress: None,
    }
}

#[test]
fn completion_window_and_arrival_cohort_have_independent_counts() {
    let bounds = Bounds {
        start: Duration::from_nanos(10),
        end: Some(Duration::from_nanos(20)),
        deadline: Some(Duration::from_nanos(30)),
    };
    let mut summary = Samples::new(bounds, 2);
    summary.record(sample(9, 2, Outcome::Completed));
    summary.record(sample(12, 2, Outcome::Completed));
    summary.record(sample(19, 3, Outcome::Completed));
    summary.record(sample(18, 12, Outcome::Pending));
    assert_eq!([2, 0, 0, 0, 1, 0], summary.counts);
    assert_eq!(2, summary.window_completed);
    assert_eq!(3, summary.all_completed);
    assert_eq!(1, summary.warmup_requests);
    let report = summary.write_report(Duration::from_nanos(30));
    let expected = serde_json::json!(200000000.0);
    assert_eq!(expected, report["operations_per_second"]);
}

#[test]
fn raw_traces_and_error_messages_are_bounded() {
    let mut summary = Samples::new(
        Bounds {
            start: Duration::ZERO,
            end: None,
            deadline: None,
        },
        3,
    );
    for operation in 0..1000 {
        let mut request = sample(0, 10, Outcome::Failed(operation.to_string()));
        request.phases = Some(Box::new(super::PhaseTrace {
            operation,
            completed_ns: 10,
            ..Default::default()
        }));
        summary.record(request);
    }
    assert_eq!(3, summary.traces.len());
    assert_eq!(64, summary.errors.len());
    assert_eq!(936, summary.errors_omitted);
    assert_eq!(1000, summary.phase_count);
    let phases = summary.phase_report();
    let expected = serde_json::json!(997);
    assert_eq!(expected, phases["traces_dropped"]);
}
#[test]
fn unmeasured_workers_allocate_no_histogram_buckets() {
    let mut summary = Samples::new(
        Bounds {
            start: Duration::ZERO,
            end: None,
            deadline: None,
        },
        0,
    );
    summary.record_unmeasured();
    let actual = summary.memory(1);
    let expected = serde_json::json!(0);
    assert_eq!(expected, actual["histogram_bytes"]);
    assert_eq!(1, summary.completed());
}

#[test]
fn failed_completions_use_the_completion_window() {
    let bounds = Bounds {
        start: Duration::from_nanos(10),
        end: Some(Duration::from_nanos(20)),
        deadline: None,
    };
    let mut summary = Samples::new(bounds, 0);
    summary.record(sample(19, 6, Outcome::Failed("failure".into())));
    let report = summary.read_report(Duration::from_nanos(25));
    let expected = serde_json::json!(0.0);
    assert_eq!(expected, report["failed"]["operations_per_second"]);
    let timing = summary.timing();
    let expected = serde_json::json!(1);
    assert_eq!(expected, timing["failed"]);
    let expected = serde_json::json!({"failure":1});
    assert_eq!(expected, timing["errors"]);
}
#[test]
fn retained_traces_cover_burst_positions() {
    let mut summary = Samples::new(
        Bounds {
            start: Duration::ZERO,
            end: None,
            deadline: None,
        },
        512,
    );
    for operation in 0..65536 {
        summary.retain_trace(super::PhaseTrace {
            operation,
            ..Default::default()
        });
    }
    let actual: std::collections::BTreeSet<_> = summary.traces.values().map(|trace| trace.operation % 64).collect();
    let expected: std::collections::BTreeSet<_> = (0..64).collect();
    assert_eq!(expected, actual);
}

#[test]
fn completed_count_runs_keep_batch_counts_and_request_latencies() {
    let bounds = Bounds {
        start: Duration::ZERO,
        end: None,
        deadline: None,
    };
    let mut latencies = super::Histogram::default();
    latencies.record(20);
    latencies.record(30);
    let summary = Samples::from_completed(bounds, latencies, 8);
    let summary = summary.with_batch_size(4);
    let actual = summary.write_report(Duration::from_secs(1));
    let expected = serde_json::json!({"operations":8,"operations_per_second":8.0,
        "completed_in_window":8,"operations_per_second_through_drain":8.0,
        "whole_workload_operations_per_second":8.0,
        "latency_unit":"request_or_batch","latency":{"samples":2,"mean_ns":25,"p50_ns":20,"p95_ns":30,"p99_ns":30}});
    assert_eq!(expected, actual);
}

#[test]
fn partial_batch_acknowledgements_survive_deadlines_and_stream_errors() {
    let bounds = Bounds {
        start: Duration::from_nanos(10),
        end: Some(Duration::from_nanos(20)),
        deadline: None,
    };
    let cases = [
        (Outcome::Pending, false, [2, 0, 0, 0, 2, 0], [0, 0, 0, 0, 1, 0]),
        (Outcome::TimedOut, false, [2, 0, 2, 0, 0, 0], [0, 0, 1, 0, 0, 0]),
        (Outcome::Failed("stream closed".into()), false, [2, 0, 0, 0, 2, 0], [
            0, 1, 0, 0, 0, 0,
        ]),
        (Outcome::Failed("entry error".into()), true, [2, 1, 0, 0, 1, 0], [
            0, 1, 0, 0, 0, 0,
        ]),
    ];
    for (outcome, entry_error, operations, requests) in cases {
        let origin = std::time::Instant::now();
        let mut progress = super::super::write_progress::WriteProgress::new(origin, bounds, 4);
        progress.acknowledge_at(true, origin + Duration::from_nanos(14));
        progress.acknowledge_at(true, origin + Duration::from_nanos(22));
        if entry_error {
            progress.acknowledge_at(false, origin + Duration::from_nanos(16));
        }
        let mut request = sample(12, 18, outcome);
        request.invoked = true;
        request.write_progress = Some(Box::new(progress));
        let summary = Samples::new(bounds, 0);
        let mut summary = summary.with_batch_size(4);
        summary.record(request);
        let timing = summary.timing();
        assert_eq!(operations, summary.counts);
        assert_eq!(2, summary.completed());
        assert_eq!(1, summary.window_completed);
        assert_eq!(0, summary.successful.count);
        let expected = serde_json::json!(operations[2] + operations[4]);
        assert_eq!(expected, timing["unanswered_invoked"]);
        let actual = summary.request_counts();
        assert_eq!([requests; 2], actual);
        let mut aggregate = Samples::new(bounds, 0);
        aggregate.merge(summary);
        let actual = aggregate.timing();
        assert_eq!(timing, actual);
    }
}

#[test]
fn warmup_batch_acknowledgements_count_in_the_completion_window() {
    let bounds = Bounds {
        start: Duration::from_nanos(10),
        end: Some(Duration::from_nanos(20)),
        deadline: None,
    };
    let origin = std::time::Instant::now();
    let mut progress = super::super::write_progress::WriteProgress::new(origin, bounds, 4);
    progress.acknowledge_at(true, origin + Duration::from_nanos(11));
    progress.acknowledge_at(true, origin + Duration::from_nanos(19));
    progress.acknowledge_at(true, origin + Duration::from_nanos(22));
    let mut request = sample(9, 21, Outcome::Pending);
    request.write_progress = Some(Box::new(progress));
    let summary = Samples::new(bounds, 0);
    let mut summary = summary.with_batch_size(4);
    summary.record(request);
    assert_eq!([0; 6], summary.counts);
    assert_eq!([3, 0, 0, 0, 1, 0], summary.all_counts);
    assert_eq!(2, summary.window_completed);
    assert_eq!(1, summary.warmup_requests);
    assert_eq!(0, summary.successful.count);
    let requests = summary.request_counts();
    assert_eq!([[0; 6], [0, 0, 0, 0, 1, 0]], requests);
}

#[test]
fn a_completed_batch_has_one_latency_sample_and_per_entry_window_counts() {
    let bounds = Bounds {
        start: Duration::from_nanos(10),
        end: Some(Duration::from_nanos(20)),
        deadline: None,
    };
    let origin = std::time::Instant::now();
    let mut progress = super::super::write_progress::WriteProgress::new(origin, bounds, 4);
    for at in [14, 19, 22, 25] {
        progress.acknowledge_at(true, origin + Duration::from_nanos(at));
    }
    let mut request = sample(12, 13, Outcome::Completed);
    request.write_progress = Some(Box::new(progress));
    let summary = Samples::new(bounds, 0);
    let mut summary = summary.with_batch_size(4);
    summary.record(request);
    assert_eq!([4, 0, 0, 0, 0, 0], summary.counts);
    assert_eq!(2, summary.window_completed);
    assert_eq!(1, summary.successful.count);
    let requests = summary.request_counts();
    assert_eq!([[1, 0, 0, 0, 0, 0]; 2], requests);
}
