use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use super::histogram::Histogram;
use super::measurement::Bounds;
use super::request_sample::LegacyReadSample;
use super::request_sample::Outcome;
use super::request_sample::PhaseTrace;
use super::request_sample::RequestSample;

const MAX_ERRORS: usize = 64;
const PHASES: [&str; 8] = [
    "client_delay",
    "service",
    "queue_wait",
    "generator_delay",
    "authorization",
    "barrier_wait",
    "query_lock_wait",
    "query",
];

/// Per-worker summaries retain bounded histograms and a bounded sample of raw traces.
pub(super) struct Samples {
    bounds: Bounds,
    trace_limit: usize,
    counts: [u64; 6],
    all_counts: [u64; 6],
    pub(super) successful: Histogram,
    failed: Histogram,
    retries: u64,
    errors: BTreeMap<String, u64>,
    errors_omitted: u64,
    phases: Option<Box<[Histogram; 8]>>,
    traces: BTreeMap<u64, PhaseTrace>,
    phase_count: u64,
    pub(super) window_completed: u64,
    window_failed: u64,
    pub(super) all_completed: u64,
    warmup_requests: u64,
    unanswered_invoked: u64,
    unanswered_not_invoked: u64,
    worker_histogram_upper_bound: usize,
    worker_trace_payload_upper_bound: usize,
}

impl Samples {
    pub(super) fn new(bounds: Bounds, trace_limit: usize) -> Self {
        Self {
            bounds,
            trace_limit,
            counts: [0; 6],
            all_counts: [0; 6],
            successful: Histogram::default(),
            failed: Histogram::default(),
            retries: 0,
            errors: BTreeMap::new(),
            errors_omitted: 0,
            phases: None,
            traces: BTreeMap::new(),
            phase_count: 0,
            window_completed: 0,
            window_failed: 0,
            all_completed: 0,
            warmup_requests: 0,
            unanswered_invoked: 0,
            unanswered_not_invoked: 0,
            worker_histogram_upper_bound: 0,
            worker_trace_payload_upper_bound: 0,
        }
    }

    pub(super) fn record(&mut self, sample: RequestSample) {
        let outcome = match sample.outcome {
            Outcome::Completed => 0,
            Outcome::Failed(_) => 1,
            Outcome::TimedOut => 2,
            Outcome::Rejected => 3,
            Outcome::Pending => 4,
            Outcome::Missed => 5,
        };
        self.all_counts[outcome] += 1;
        let completed = sample.scheduled_ns.saturating_add(sample.latency_ns);
        if matches!(&sample.outcome, Outcome::Failed(_)) {
            self.window_failed += u64::from(self.bounds.contains(completed));
        }
        if sample.outcome == Outcome::Completed {
            self.all_completed += 1;
            self.window_completed += u64::from(self.bounds.contains(completed));
        }
        if !self.bounds.contains(sample.scheduled_ns) {
            self.warmup_requests += 1;
            return;
        }
        if sample.outcome == Outcome::Pending || sample.outcome == Outcome::TimedOut {
            if sample.invoked {
                self.unanswered_invoked += 1;
            } else {
                self.unanswered_not_invoked += 1;
            }
        }
        self.retries += sample.retries;
        self.record_outcome(sample.outcome, sample.latency_ns);
        if let Some(trace) = sample.phases {
            self.record_phase(*trace);
        }
    }

    pub(super) fn record_read(&mut self, sample: LegacyReadSample) {
        match sample.status {
            Ok(()) => self.record_latency(sample.latency_ns),
            Err(error) => {
                self.all_counts[1] += 1;
                self.window_failed += 1;
                self.record_outcome(Outcome::Failed(error), sample.latency_ns);
            }
        }
    }

    pub(super) fn record_latency(&mut self, latency: u64) {
        self.counts[0] += 1;
        self.all_counts[0] += 1;
        self.all_completed += 1;
        self.window_completed += 1;
        self.successful.record(latency);
    }

    pub(super) fn record_unmeasured(&mut self) {
        self.counts[0] += 1;
        self.all_counts[0] += 1;
        self.all_completed += 1;
        self.window_completed += 1;
    }

    fn record_outcome(&mut self, outcome: Outcome, latency: u64) {
        match outcome {
            Outcome::Completed => {
                self.counts[0] += 1;
                self.successful.record(latency);
            }
            Outcome::Failed(error) => {
                self.counts[1] += 1;
                self.failed.record(latency);
                self.record_error(error, 1);
            }
            Outcome::TimedOut => self.counts[2] += 1,
            Outcome::Rejected => self.counts[3] += 1,
            Outcome::Pending => self.counts[4] += 1,
            Outcome::Missed => self.counts[5] += 1,
        }
    }

    fn record_error(&mut self, error: String, count: u64) {
        if self.errors.contains_key(&error) || self.errors.len() < MAX_ERRORS {
            *self.errors.entry(error).or_default() += count;
        } else {
            self.errors_omitted += count;
        }
    }

    fn record_phase(&mut self, trace: PhaseTrace) {
        let phases = self.phases.get_or_insert_with(|| Box::new(std::array::from_fn(|_| Histogram::default())));
        let values = [
            Some(trace.dispatched_ns - trace.scheduled_ns),
            Some(trace.completed_ns - trace.dispatched_ns),
            trace.released_ns.map(|at| trace.dispatched_ns - at),
            trace.released_ns.map(|at| at - trace.scheduled_ns),
            trace.authorized_ns.map(|at| at - trace.dispatched_ns),
            trace.barrier_ready_ns.zip(trace.authorized_ns).map(|(end, start)| end - start),
            trace.query_lock_ns.zip(trace.barrier_ready_ns).map(|(end, start)| end - start),
            trace.query_lock_ns.map(|at| trace.completed_ns - at),
        ];
        for (histogram, value) in phases.iter_mut().zip(values) {
            if let Some(value) = value {
                histogram.record(value);
            }
        }
        self.phase_count += 1;
        self.retain_trace(trace);
    }

    fn retain_trace(&mut self, trace: PhaseTrace) {
        if self.trace_limit == 0 {
            return;
        }
        let priority = super::arrival::random_bits(trace.operation ^ 0xa0761d6478bd642f);
        if self.traces.len() == self.trace_limit {
            let largest = *self.traces.last_key_value().unwrap().0;
            if priority >= largest {
                return;
            }
            self.traces.pop_last();
        }
        self.traces.insert(priority, trace);
    }

    pub(super) fn merge(&mut self, other: Self) {
        self.worker_histogram_upper_bound = self.worker_histogram_upper_bound.max(self.histogram_bytes());
        self.worker_histogram_upper_bound += other.worker_histogram_upper_bound.max(other.histogram_bytes());
        self.worker_trace_payload_upper_bound =
            self.worker_trace_payload_upper_bound.max(self.traces.len() * size_of::<PhaseTrace>());
        self.worker_trace_payload_upper_bound +=
            other.worker_trace_payload_upper_bound.max(other.traces.len() * size_of::<PhaseTrace>());
        for (target, source) in self.counts.iter_mut().zip(other.counts) {
            *target += source;
        }
        for (target, source) in self.all_counts.iter_mut().zip(other.all_counts) {
            *target += source;
        }
        self.successful.merge(other.successful);
        self.failed.merge(other.failed);
        self.retries += other.retries;
        self.errors_omitted += other.errors_omitted;
        self.window_completed += other.window_completed;
        self.all_completed += other.all_completed;
        self.window_failed += other.window_failed;
        self.warmup_requests += other.warmup_requests;
        self.unanswered_invoked += other.unanswered_invoked;
        self.unanswered_not_invoked += other.unanswered_not_invoked;
        for (error, count) in other.errors {
            self.record_error(error, count);
        }
        if let Some(other_phases) = other.phases {
            let phases = self.phases.get_or_insert_with(|| Box::new(std::array::from_fn(|_| Histogram::default())));
            for (target, source) in phases.iter_mut().zip(*other_phases) {
                target.merge(source);
            }
        }
        self.phase_count += other.phase_count;
        for trace in other.traces.into_values() {
            self.retain_trace(trace);
        }
    }

    pub(super) fn timing(&self, entries: u64) -> Value {
        let counts = self.counts.map(|count| count * entries);
        let offered: u64 = counts.iter().sum();
        let errors: BTreeMap<_, _> = self.errors.iter().map(|(error, count)| (error, count * entries)).collect();
        serde_json::json!({"offered":offered,"admitted":offered-counts[3]-counts[5],"completed":counts[0],
            "failed":counts[1],"timed_out":counts[2],"rejected":counts[3],"pending":counts[4],
            "missed":counts[5],"all_phase_outcomes":{
                "completed":self.all_counts[0]*entries,"failed":self.all_counts[1]*entries,"timed_out":self.all_counts[2]*entries,
                "rejected":self.all_counts[3]*entries,"pending":self.all_counts[4]*entries,"missed":self.all_counts[5]*entries},
            "unanswered_invoked":self.unanswered_invoked*entries,"unanswered_not_invoked":self.unanswered_not_invoked*entries,
            "retries":self.retries,"errors":errors,"errors_omitted":self.errors_omitted*entries,
            "warmup_requests":self.warmup_requests,"phases":self.phase_report()})
    }

    fn phase_report(&self) -> Value {
        let mut result = serde_json::Map::new();
        for (index, name) in PHASES.iter().enumerate() {
            let value = self.phases.as_ref().map_or(Value::Null, |phases| phases[index].latency());
            result.insert((*name).to_string(), value);
        }
        let traces: Vec<_> = self.traces.values().collect();
        result.insert("samples".into(), self.phase_count.into());
        result.insert(
            "traces_dropped".into(),
            (self.phase_count - self.traces.len() as u64).into(),
        );
        result.insert("traces".into(), serde_json::to_value(traces).unwrap());
        Value::Object(result)
    }

    pub(super) fn write_report(&self, elapsed: Duration, entries: u64) -> Value {
        let seconds = self.bounds.seconds(elapsed);
        serde_json::json!({"operations":self.counts[0]*entries,
            "operations_per_second":self.window_completed as f64*entries as f64/seconds,
            "completed_in_window":self.window_completed*entries,
            "operations_per_second_through_drain":self.counts[0] as f64*entries as f64/elapsed.saturating_sub(self.bounds.start).as_secs_f64(),
            "whole_workload_operations_per_second":self.all_completed as f64*entries as f64/elapsed.as_secs_f64(),
            "latency":self.successful.latency()})
    }

    pub(super) fn read_report(&self, elapsed: Duration) -> Value {
        let successful = self.write_report(elapsed, 1);
        let failed = serde_json::json!({"operations":self.counts[1],
            "operations_per_second":self.window_failed as f64/self.bounds.seconds(elapsed),
            "completed_in_window":self.window_failed,"latency":self.failed.latency()});
        serde_json::json!({"successful":successful,"failed":failed,"errors":self.errors,"timing":self.timing(1)})
    }

    fn histogram_bytes(&self) -> usize {
        let phase_bytes = self.phases.as_ref().map_or(0, |phases| phases.iter().map(Histogram::bytes).sum());
        self.successful.bytes() + self.failed.bytes() + phase_bytes
    }

    pub(super) fn memory(&self) -> Value {
        let histograms = self.histogram_bytes();
        let traces = self.traces.len() * size_of::<PhaseTrace>();
        serde_json::json!({"histogram_bytes":histograms,"storage_scope":"aggregate_at_report",
            "worker_histogram_bytes_upper_bound":self.worker_histogram_upper_bound.max(histograms),
            "worker_trace_payload_bytes_upper_bound":self.worker_trace_payload_upper_bound.max(traces),
            "worker_storage_scope":"sum_of_worker_retained_peak_payloads_excludes_container_overhead",
            "trace_selection":"per_worker_mixed_hash_priority_sample",
            "retained_traces":self.traces.len(),"trace_payload_bytes":self.traces.len()*size_of::<PhaseTrace>(),
            "trace_limit":self.trace_limit,"percentile_relative_bucket_width":1.0/64.0,
            "percentiles":"bucket_upper_bound","exact_counts_and_mean":true})
    }

    pub(super) fn completed(&self) -> u64 {
        self.counts[0]
    }
    #[cfg(test)]
    pub(super) fn request_count(&self) -> u64 {
        self.counts.iter().sum()
    }
}

#[cfg(test)]
mod tests {
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
        let report = summary.write_report(Duration::from_nanos(30), 1);
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
        let actual = summary.memory();
        let expected = serde_json::json!(0);
        assert_eq!(expected, actual["histogram_bytes"]);
        assert_eq!(1, summary.completed());
    }

    #[test]
    fn failed_completions_and_batch_errors_keep_their_units() {
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
        let timing = summary.timing(4);
        let expected = serde_json::json!(4);
        assert_eq!(expected, timing["failed"]);
        let expected = serde_json::json!({"failure":4});
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
}
