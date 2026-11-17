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
    entries_per_request: u64,
    request_outcomes: Option<Box<[[u64; 6]; 2]>>,
    worker_request_counter_upper_bound: usize,
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
            entries_per_request: 1,
            request_outcomes: None,
            worker_request_counter_upper_bound: 0,
        }
    }

    pub(super) fn record(&mut self, sample: RequestSample) {
        let outcome = sample.outcome.index();
        let measured = self.bounds.contains(sample.scheduled_ns);
        self.record_request(outcome, measured);
        let counts = match &sample.write_progress {
            Some(progress) => {
                assert_eq!(self.entries_per_request, progress.entries);
                progress.outcomes(&sample.outcome)
            }
            None => {
                let mut counts = [0; 6];
                counts[outcome] = self.entries_per_request;
                counts
            }
        };
        for (target, count) in self.all_counts.iter_mut().zip(counts) {
            *target += count;
        }
        self.all_completed += counts[0];
        let completed = sample.scheduled_ns.saturating_add(sample.latency_ns);
        match &sample.write_progress {
            Some(progress) => {
                self.window_completed += progress.window_completed;
                self.window_failed += progress.window_failed;
            }
            None if self.bounds.contains(completed) => {
                self.window_completed += counts[0];
                self.window_failed += counts[1];
            }
            None => {}
        }
        if !measured {
            self.warmup_requests += 1;
            return;
        }
        for (target, count) in self.counts.iter_mut().zip(counts) {
            *target += count;
        }
        let unanswered = counts[2] + counts[4];
        if sample.invoked {
            self.unanswered_invoked += unanswered;
        } else {
            self.unanswered_not_invoked += unanswered;
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
                self.counts[1] += 1;
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
        self.counts[0] += self.entries_per_request;
        self.all_counts[0] += self.entries_per_request;
        self.all_completed += self.entries_per_request;
        self.window_completed += self.entries_per_request;
    }

    pub(super) fn from_completed(bounds: Bounds, successful: Histogram, operations: u64) -> Self {
        let mut samples = Self::new(bounds, 0);
        samples.counts[0] = operations;
        samples.all_counts[0] = operations;
        samples.all_completed = operations;
        samples.window_completed = operations;
        samples.successful = successful;
        samples
    }

    fn record_outcome(&mut self, outcome: Outcome, latency: u64) {
        match outcome {
            Outcome::Completed => self.successful.record(latency),
            Outcome::Failed(error) => {
                self.failed.record(latency);
                self.record_error(error, 1);
            }
            Outcome::TimedOut | Outcome::Rejected | Outcome::Pending | Outcome::Missed => {}
        }
    }

    pub(super) fn with_batch_size(mut self, entries: u64) -> Self {
        assert!(entries > 0, "requests contain at least one operation");
        self.entries_per_request = entries;
        self
    }

    fn request_counts(&self) -> [[u64; 6]; 2] {
        match &self.request_outcomes {
            Some(counts) => **counts,
            None => [
                self.counts.map(|count| count / self.entries_per_request),
                self.all_counts.map(|count| count / self.entries_per_request),
            ],
        }
    }

    fn record_request(&mut self, outcome: usize, measured: bool) {
        if self.entries_per_request == 1 {
            return;
        }
        let before = self.request_counts();
        let counts = self.request_outcomes.get_or_insert_with(|| Box::new(before));
        counts[1][outcome] += 1;
        if measured {
            counts[0][outcome] += 1;
        }
    }

    fn merge_requests(&mut self, other: &Self) {
        if self.request_outcomes.is_none() && other.request_outcomes.is_none() {
            return;
        }
        let before = self.request_counts();
        let target = self.request_outcomes.get_or_insert_with(|| Box::new(before));
        for (target, source) in target.iter_mut().zip(other.request_counts()) {
            for (target, source) in target.iter_mut().zip(source) {
                *target += source;
            }
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
        let empty = self.all_counts.iter().all(|count| *count == 0);
        if empty {
            self.entries_per_request = other.entries_per_request;
        }
        assert_eq!(self.entries_per_request, other.entries_per_request);
        self.worker_request_counter_upper_bound =
            self.worker_request_counter_upper_bound.max(self.request_counter_bytes());
        self.worker_request_counter_upper_bound +=
            other.worker_request_counter_upper_bound.max(other.request_counter_bytes());
        self.merge_requests(&other);
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

    pub(super) fn timing(&self) -> Value {
        let counts = self.counts;
        let offered: u64 = counts.iter().sum();
        let requests = self.request_counts();
        serde_json::json!({"offered":offered,"admitted":offered-counts[3]-counts[5],"completed":counts[0],
            "failed":counts[1],"timed_out":counts[2],"rejected":counts[3],"pending":counts[4],
            "missed":counts[5],"all_phase_outcomes":outcomes(self.all_counts),
            "request_outcomes":outcomes(requests[0]),"all_phase_request_outcomes":outcomes(requests[1]),
            "operations_per_request":self.entries_per_request,"outcome_unit":"operation","latency_unit":"request_or_batch",
            "unanswered_invoked":self.unanswered_invoked,"unanswered_not_invoked":self.unanswered_not_invoked,
            "retries":self.retries,"errors":self.errors,"errors_omitted":self.errors_omitted,"errors_unit":"request_or_batch",
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

    pub(super) fn write_report(&self, elapsed: Duration) -> Value {
        let seconds = self.bounds.seconds(elapsed);
        serde_json::json!({"operations":self.counts[0],
            "operations_per_second":self.window_completed as f64/seconds,
            "completed_in_window":self.window_completed,
            "operations_per_second_through_drain":self.counts[0] as f64/elapsed.saturating_sub(self.bounds.start).as_secs_f64(),
            "whole_workload_operations_per_second":self.all_completed as f64/elapsed.as_secs_f64(),
            "latency":self.successful.latency(),"latency_unit":"request_or_batch"})
    }

    pub(super) fn read_report(&self, elapsed: Duration) -> Value {
        let successful = self.write_report(elapsed);
        let failed = serde_json::json!({"operations":self.counts[1],
            "operations_per_second":self.window_failed as f64/self.bounds.seconds(elapsed),
            "completed_in_window":self.window_failed,"latency":self.failed.latency()});
        serde_json::json!({"successful":successful,"failed":failed,"errors":self.errors,"timing":self.timing()})
    }

    fn histogram_bytes(&self) -> usize {
        let phase_bytes = self.phases.as_ref().map_or(0, |phases| phases.iter().map(Histogram::bytes).sum());
        self.successful.bytes() + self.failed.bytes() + phase_bytes
    }

    fn request_counter_bytes(&self) -> usize {
        self.request_outcomes.as_ref().map_or(0, |_| size_of::<[[u64; 6]; 2]>())
    }

    pub(super) fn memory(&self, workers: usize) -> Value {
        let histograms = self.histogram_bytes();
        let traces = self.traces.len() * size_of::<PhaseTrace>();
        serde_json::json!({"histogram_bytes":histograms,"storage_scope":"aggregate_at_report",
            "aggregate_summary_payload_bytes":size_of::<Self>(),
            "worker_summary_allocations":workers,"worker_summary_payload_bytes_upper_bound":workers*size_of::<Self>(),
            "request_outcome_counter_bytes":self.request_counter_bytes(),
            "worker_request_outcome_counter_bytes_upper_bound":self.worker_request_counter_upper_bound.max(self.request_counter_bytes()),
            "worker_histogram_bytes_upper_bound":self.worker_histogram_upper_bound.max(histograms),
            "worker_trace_payload_bytes_upper_bound":self.worker_trace_payload_upper_bound.max(traces),
            "worker_storage_scope":"sum_of_worker_retained_peak_payloads_excludes_allocator_and_tokio_task_overhead",
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

fn outcomes(counts: [u64; 6]) -> Value {
    serde_json::json!({"completed":counts[0],"failed":counts[1],"timed_out":counts[2],
        "rejected":counts[3],"pending":counts[4],"missed":counts[5]})
}

#[cfg(test)]
#[path = "summary/summary_test.rs"]
mod summary_test;
