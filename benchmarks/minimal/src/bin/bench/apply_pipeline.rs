use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Instant;

use bench_minimal::apply_pipeline::PipelineEvent;
use bench_minimal::network::Router;
use bench_minimal::store::TypeConfig;
use openraft::alias::EntryOf;
use serde::Serialize;
use serde_json::Value;

use super::histogram::Histogram;
use super::measurement::Bounds;
use crate::BenchConfig;

/// Every aggregate is online; only a capped suffix of complete spans is retained.
pub(super) struct Pipeline {
    epoch: Arc<OnceLock<Instant>>,
    bounds: Bounds,
    trace_limit: usize,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    reading: Option<Reading>,
    applying: Option<Applying>,
    pending: Buffered,
    peak_entries: u64,
    peak_payload_bytes: u64,
    reads: [ReadStats; 2],
    apply_duration: Histogram,
    reads_per_apply: Histogram,
    applies_with_extensions: u64,
    counters: Counters,
    traces: VecDeque<Trace>,
    traces_dropped: u64,
}

#[derive(Default, Serialize)]
struct Counters {
    read_started: u64,
    read_completed: u64,
    read_failed: u64,
    read_cancelled: u64,
    read_entries: u64,
    apply_started: u64,
    apply_completed: u64,
    apply_ended_without_success: u64,
    applied_entries: u64,
    entries_assigned_to_unsuccessful_applies: u64,
    unpaired_events: u64,
    overlapping_reads: u64,
    overlapping_applies: u64,
    range_or_count_mismatches: u64,
    unknown_read_entry_counts: u64,
    unknown_read_ranges: u64,
}

#[derive(Clone, Default, Serialize)]
struct Buffered {
    first: Option<u64>,
    end: Option<u64>,
    reads: u64,
    entries: u64,
    payload_bytes: u64,
    unknown_reads: u64,
}

struct Reading {
    started: Instant,
    range: Option<Range<u64>>,
    overlap_started: Option<Instant>,
    overlapped_ns: u64,
    overlap_in_window_ns: u64,
}

struct Applying {
    started: Instant,
    ended: Option<Instant>,
    buffered: Buffered,
    published: Option<(u64, u64)>,
}

struct FinishedRead {
    read: Reading,
    at: Instant,
    entries: Option<(u64, u64)>,
    status: &'static str,
    extension: bool,
}

#[derive(Default)]
struct ReadStats {
    duration: Histogram,
    unoverlapped: Histogram,
    entries: Histogram,
    time_in_window_ns: u64,
    overlap_in_window_ns: u64,
}

#[derive(Serialize)]
struct Trace {
    kind: &'static str,
    status: &'static str,
    start_ns: i128,
    end_ns: i128,
    range: Option<Range<u64>>,
    entries_and_payload_bytes: Option<(u64, u64)>,
    overlapped_ns: u64,
}

pub(super) async fn install(
    router: &Router,
    config: &BenchConfig,
    epoch: Arc<OnceLock<Instant>>,
) -> anyhow::Result<BTreeMap<u64, Arc<Pipeline>>> {
    let mut nodes = BTreeMap::new();
    if !config.measurement.measure_apply_pipeline {
        return Ok(nodes);
    }
    for node in &config.members {
        let trace_limit = config.measurement.trace_limit / config.members.len();
        let probe = Arc::new(Pipeline::new(epoch.clone(), config.measurement.bounds(), trace_limit));
        let observing = probe.clone();
        router.observe_apply_pipeline(*node, Arc::new(move |event| observing.observe(event))).await?;
        nodes.insert(*node, probe);
    }
    Ok(nodes)
}

impl Pipeline {
    fn new(epoch: Arc<OnceLock<Instant>>, bounds: Bounds, trace_limit: usize) -> Self {
        Self {
            epoch,
            bounds,
            trace_limit,
            state: Mutex::new(State::default()),
        }
    }

    fn observe(&self, event: PipelineEvent) {
        let mut state = self.state.lock().unwrap();
        match event {
            PipelineEvent::ReadStarted { at, range } => state.start_read(at, range),
            PipelineEvent::ReadFinished { .. } => state.finish_read(event, self),
            PipelineEvent::ApplyStarted(at) => state.start_apply(at),
            PipelineEvent::Applied { at: _, index, entries } => state.published(index, entries),
            PipelineEvent::ApplyFinished(at) => state.finish_apply(at, self),
        }
    }

    fn contains(&self, at: Instant) -> bool {
        let Some(epoch) = self.epoch.get() else { return false };
        let Some(elapsed) = at.checked_duration_since(*epoch) else {
            return false;
        };
        self.bounds.contains(elapsed.as_nanos() as u64)
    }

    fn clipped(&self, range: Range<Instant>) -> Option<Range<Instant>> {
        let epoch = self.epoch.get()?;
        let start = range.start.max(*epoch + self.bounds.start);
        let end = self.bounds.end.map_or(range.end, |end| range.end.min(*epoch + end));
        Some(start..end)
    }

    fn offset(&self, at: Instant) -> i128 {
        let epoch = *self.epoch.get().expect("a measured span has an epoch");
        match at.checked_duration_since(epoch) {
            Some(elapsed) => elapsed.as_nanos() as i128,
            None => -(epoch.duration_since(at).as_nanos() as i128),
        }
    }

    pub(super) fn report(&self) -> Value {
        let state = self.state.lock().unwrap();
        serde_json::json!({"schema_version":1,"initial_reads":state.reads[0].report(),"extension_reads":state.reads[1].report(),
            "apply_duration_ns":state.apply_duration.latency(),"reads_per_apply":state.reads_per_apply.report(),
            "applies_with_extensions":state.applies_with_extensions,
            "conservation":state.conservation(),"all_observed_phase_counters":state.counters,
            "read_not_yet_consumed":state.pending,"inflight":state.inflight(self),"buffers":state.buffers(),
            "traces":state.traces,"trace_limit":self.trace_limit,"traces_dropped":state.traces_dropped,
            "trace_spans_seen":state.traces.len() as u64 + state.traces_dropped,
            "trace_budget_scope":"additional_global_trace_limit_budget_divided_across_nodes",
            "trace_population":"bounded_recent_spans_ending_in_measurement","histogram_relative_width_max":"1/64",
            "span_population":"store_calls_ending_in_measurement_including_straddling_calls",
            "time_population":"closed_read_spans_including_failures_and_cancellation_clipped_to_measurement; unfinished_read_time_is_excluded",
            "histogram_outcomes":"successful_calls_only; failures_and_unfinished_spans_remain_in_counters",
            "extension_definition":"additional_contiguous_entries_stream_call_before_the_same_apply",
            "assignment_validation":"completed_read_prefix_is_assigned_at_apply_start_and_verified_against_source_range_and_count",
            "read_endpoint":"entries_stream_has_materialized_its_result; worker_stream_collection_is_excluded",
            "apply_endpoint":"benchmark_apply_method_exits; successful_publication_precedes_exit",
            "diagnostic_scope":"opt_in_store_observers_can_change_scheduling; performance_uses_feature_off"})
    }
}

impl State {
    fn start_read(&mut self, at: Instant, range: Option<Range<u64>>) {
        self.counters.read_started += 1;
        if self.reading.is_some() {
            self.counters.overlapping_reads += 1;
        }
        if range.is_none() {
            self.counters.unknown_read_ranges += 1;
        }
        let applying = self.applying.as_ref().is_some_and(|apply| apply.ended.is_none());
        let overlap_started = if applying { Some(at) } else { None };
        self.reading = Some(Reading {
            started: at,
            range,
            overlap_started,
            overlapped_ns: 0,
            overlap_in_window_ns: 0,
        });
    }

    fn finish_read(&mut self, event: PipelineEvent, probe: &Pipeline) {
        let PipelineEvent::ReadFinished { at, entries, status } = event else {
            unreachable!()
        };
        let Some(mut read) = self.reading.take() else {
            self.counters.unpaired_events += 1;
            return;
        };
        read.close_overlap(at, probe);
        let extension = self.pending.reads > 0;
        let finished = FinishedRead {
            read,
            at,
            entries,
            status,
            extension,
        };
        self.record_read(finished, probe);
    }

    fn record_read(&mut self, read: FinishedRead, probe: &Pipeline) {
        self.count_read(read.status, read.entries);
        self.buffer_read(&read.read, read.entries);
        self.reads[usize::from(read.extension)].record(probe, &read);
        if probe.contains(read.at) {
            let trace = Trace::read(probe, &read);
            self.trace(trace, probe.trace_limit);
        }
    }

    fn count_read(&mut self, status: &str, entries: Option<(u64, u64)>) {
        match status {
            "completed" => self.counters.read_completed += 1,
            "failed" => self.counters.read_failed += 1,
            "cancelled_or_panicked" => self.counters.read_cancelled += 1,
            _ => unreachable!("known benchmark read outcome"),
        }
        if let Some((entries, _)) = entries {
            self.counters.read_entries += entries;
        }
    }

    fn buffer_read(&mut self, read: &Reading, counts: Option<(u64, u64)>) {
        let requested = read.range.as_ref().map(|range| range.end.saturating_sub(range.start));
        if let (Some(requested), Some((actual, _))) = (requested, counts)
            && requested != actual
        {
            self.counters.range_or_count_mismatches += 1;
        }
        if counts.is_none() {
            self.counters.unknown_read_entry_counts += 1;
        }
        if let (Some(end), Some(range)) = (self.pending.end, &read.range)
            && self.pending.reads > 0
            && range.start != end
        {
            self.counters.range_or_count_mismatches += 1;
        }
        self.pending.include(read, counts);
        self.peak_entries = self.peak_entries.max(self.pending.entries);
        self.peak_payload_bytes = self.peak_payload_bytes.max(self.pending.payload_bytes);
    }

    fn start_apply(&mut self, at: Instant) {
        self.counters.apply_started += 1;
        if self.applying.as_ref().is_some_and(|apply| apply.ended.is_none()) {
            self.counters.overlapping_applies += 1;
        }
        if let Some(read) = self.reading.as_mut()
            && read.overlap_started.is_none()
        {
            read.overlap_started = Some(at);
        }
        let buffered = self.pending.clone();
        self.pending = Buffered::default();
        self.applying = Some(Applying {
            started: at,
            ended: None,
            buffered,
            published: None,
        });
    }

    fn published(&mut self, index: u64, entries: u64) {
        let Some(apply) = self.applying.as_mut() else {
            self.counters.unpaired_events += 1;
            return;
        };
        if apply.ended.is_some() || apply.published.is_some() {
            self.counters.unpaired_events += 1;
        }
        apply.published = Some((index, entries));
    }

    fn finish_apply(&mut self, at: Instant, probe: &Pipeline) {
        if let Some(read) = self.reading.as_mut() {
            read.close_overlap(at, probe);
        }
        let Some(mut apply) = self.applying.take() else {
            self.counters.unpaired_events += 1;
            return;
        };
        apply.ended = Some(at);
        self.count_apply(&apply);
        if probe.contains(at) {
            if apply.published.is_some() {
                self.apply_duration.record(nanos(&(apply.started..at)));
                self.reads_per_apply.record(apply.buffered.reads);
                self.applies_with_extensions += u64::from(apply.buffered.reads > 1);
            }
            let trace = Trace::apply(probe, &apply, at);
            self.trace(trace, probe.trace_limit);
        }
        self.applying = Some(apply);
    }

    fn count_apply(&mut self, apply: &Applying) {
        let Some((index, entries)) = apply.published else {
            self.counters.apply_ended_without_success += 1;
            self.counters.entries_assigned_to_unsuccessful_applies += apply.buffered.entries;
            return;
        };
        self.counters.apply_completed += 1;
        self.counters.applied_entries += entries;
        if !apply.buffered.matches(index, entries) {
            self.counters.range_or_count_mismatches += 1;
        }
    }

    fn trace(&mut self, trace: Trace, limit: usize) {
        if limit == 0 {
            self.traces_dropped += 1;
            return;
        }
        if self.traces.len() == limit {
            self.traces.pop_front();
            self.traces_dropped += 1;
        }
        self.traces.push_back(trace);
    }

    fn conservation(&self) -> Value {
        let active_read = u64::from(self.reading.is_some());
        let applying = self.applying.as_ref().filter(|apply| apply.ended.is_none());
        let active_apply = u64::from(applying.is_some());
        let active_entries = applying.map_or(0, |apply| apply.buffered.entries);
        let reads = self.counters.read_completed + self.counters.read_failed + self.counters.read_cancelled;
        let applies = self.counters.apply_completed + self.counters.apply_ended_without_success;
        let assigned = self.counters.applied_entries + self.counters.entries_assigned_to_unsuccessful_applies;
        let entries = assigned + self.pending.entries + active_entries;
        serde_json::json!({"read_calls":self.counters.read_started == reads + active_read,
            "apply_calls":self.counters.apply_started == applies + active_apply,
            "read_entry_assignments":self.counters.read_entries == entries,
            "range_and_count_validation":self.counters.range_or_count_mismatches == 0,
            "known_read_entry_counts":self.counters.unknown_read_entry_counts == 0,
            "known_read_ranges":self.counters.unknown_read_ranges == 0,
            "paired_events":self.counters.unpaired_events == 0,
            "single_read_and_apply":self.counters.overlapping_reads == 0 && self.counters.overlapping_applies == 0})
    }

    fn inflight(&self, probe: &Pipeline) -> Value {
        let applying = self.applying.as_ref().filter(|apply| apply.ended.is_none());
        let started = self.reading.as_ref().map(|read| {
            let started = read.started;
            probe.offset(started)
        });
        serde_json::json!({"read":self.reading.as_ref().map(|read| &read.range),
            "read_calls":usize::from(self.reading.is_some()),"apply_calls":usize::from(applying.is_some()),
            "read_started_ns":started,"apply_assigned_reads":applying.map(|apply| &apply.buffered)})
    }

    fn buffers(&self) -> Value {
        let entry_bytes = size_of::<EntryOf<TypeConfig>>();
        let traces = self.traces.capacity() * size_of::<Trace>();
        let read_histograms: usize = self.reads.iter().map(ReadStats::bytes).sum();
        let histograms = read_histograms + self.apply_duration.bytes() + self.reads_per_apply.bytes();
        serde_json::json!({"observed_peak_read_not_yet_consumed_entries":self.peak_entries,
            "observed_peak_entry_struct_bytes":self.peak_entries * entry_bytes as u64,
            "observed_peak_referenced_payload_bytes":self.peak_payload_bytes,
            "peak_scope":"independent_maxima_over_all_observed_phases_including_warmup_and_drain",
            "byte_scope":"materialized_entries_not_yet_assigned_to_apply; excludes_active_reads,Vec_capacity,responders,allocator_and_shared_payload_ownership",
            "observer_histogram_bytes":histograms,"observer_trace_buffer_bytes":traces})
    }
}

impl ReadStats {
    fn record(&mut self, probe: &Pipeline, read: &FinishedRead) {
        let span = read.read.started..read.at;
        if let Some(clipped) = probe.clipped(span.clone()) {
            self.time_in_window_ns += nanos(&clipped);
            self.overlap_in_window_ns += read.read.overlap_in_window_ns;
        }
        if let Some((entries, _)) = read.entries
            && probe.contains(span.end)
        {
            let total = nanos(&span);
            let overlap = read.read.overlapped_ns;
            self.duration.record(total);
            self.unoverlapped.record(total.saturating_sub(overlap));
            self.entries.record(entries);
        }
    }

    fn report(&self) -> Value {
        serde_json::json!({"completed_calls":self.duration.count,"duration_ns":self.duration.latency(),
            "unoverlapped_ns":self.unoverlapped.latency(),"entries":self.entries.report(),
            "time_in_window_ns":self.time_in_window_ns,"overlap_in_window_ns":self.overlap_in_window_ns,
            "unoverlapped_time_in_window_ns":self.time_in_window_ns.saturating_sub(self.overlap_in_window_ns)})
    }

    fn bytes(&self) -> usize {
        self.duration.bytes() + self.unoverlapped.bytes() + self.entries.bytes()
    }
}

impl Trace {
    fn read(probe: &Pipeline, read: &FinishedRead) -> Self {
        let kind = if read.extension {
            "extension_read"
        } else {
            "initial_read"
        };
        let overlap = read.read.overlapped_ns;
        Self {
            kind,
            status: read.status,
            start_ns: probe.offset(read.read.started),
            end_ns: probe.offset(read.at),
            range: read.read.range.clone(),
            entries_and_payload_bytes: read.entries,
            overlapped_ns: overlap,
        }
    }

    fn apply(probe: &Pipeline, apply: &Applying, end: Instant) -> Self {
        let range = apply.buffered.range();
        let counts = apply.published.map(|(_, entries)| {
            let payload_bytes = apply.buffered.payload_bytes;
            (entries, payload_bytes)
        });
        let status = if apply.published.is_some() {
            "completed"
        } else {
            "ended_without_success"
        };
        Self {
            kind: "apply",
            status,
            start_ns: probe.offset(apply.started),
            end_ns: probe.offset(end),
            range,
            entries_and_payload_bytes: counts,
            overlapped_ns: 0,
        }
    }
}

impl Buffered {
    fn include(&mut self, read: &Reading, counts: Option<(u64, u64)>) {
        if self.reads == 0 {
            self.first = read.range.as_ref().map(|range| range.start);
        }
        self.end = read.range.as_ref().map(|range| range.end);
        self.reads += 1;
        if let Some((entries, payload)) = counts {
            self.entries += entries;
            self.payload_bytes += payload;
        } else {
            self.unknown_reads += 1;
        }
    }

    fn range(&self) -> Option<Range<u64>> {
        let first = self.first?;
        let end = self.end?;
        Some(first..end)
    }

    fn matches(&self, index: u64, entries: u64) -> bool {
        let count_matches = self.unknown_reads > 0 || entries == self.entries;
        let range_matches = match (self.first, self.end) {
            (Some(first), Some(end)) => first.checked_add(entries) == Some(end) && index.checked_add(1) == Some(end),
            _ => true,
        };
        count_matches && range_matches && self.reads > 0
    }
}

impl Reading {
    fn close_overlap(&mut self, at: Instant, probe: &Pipeline) {
        let Some(started) = self.overlap_started else { return };
        self.overlap_started = None;
        let overlap = started..at;
        self.overlapped_ns += nanos(&overlap);
        if let Some(clipped) = probe.clipped(overlap) {
            self.overlap_in_window_ns += nanos(&clipped);
        }
    }
}

fn nanos(range: &Range<Instant>) -> u64 {
    range.end.saturating_duration_since(range.start).as_nanos() as u64
}

#[cfg(test)]
mod tests {
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
        probe.observe(PipelineEvent::ApplyFinished(ended));
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
        assert_eq!(2, report["initial_reads"]["time_in_window_ns"]);
        assert_eq!(0, report["initial_reads"]["unoverlapped_time_in_window_ns"]);
        assert_eq!(3, report["extension_reads"]["unoverlapped_time_in_window_ns"]);
        assert_eq!(1, report["reads_per_apply"]["samples"]);
        assert_eq!(1, report["traces_dropped"]);
        assert_eq!(300, report["buffers"]["observed_peak_referenced_payload_bytes"]);
    }

    #[test]
    fn an_unfinished_prefetch_overruns_the_completed_apply() {
        let probe = fixture(8);
        read(&probe, 1..4, 0..1);
        start_apply(&probe, 5);
        start_read(&probe, 6, 1..2);
        finish_apply(&probe, 10, 0..1);
        finish_read(&probe, 15, 1);
        let report = probe.report();
        assert_eq!(5, report["initial_reads"]["unoverlapped_ns"]["p50_ns"]);
        assert_eq!(5, report["initial_reads"]["time_in_window_ns"]);
        assert_eq!(0, report["initial_reads"]["overlap_in_window_ns"]);
        assert_eq!(true, report["conservation"]["read_entry_assignments"]);
    }

    #[test]
    fn a_read_finishing_in_drain_keeps_its_window_time() {
        let probe = fixture(0);
        read(&probe, 5..25, 0..2);
        let report = probe.report();
        assert_eq!(10, report["initial_reads"]["time_in_window_ns"]);
        assert_eq!(0, report["initial_reads"]["completed_calls"]);
        assert_eq!(Value::Null, report["initial_reads"]["duration_ns"]);
        assert_eq!(2, report["read_not_yet_consumed"]["entries"]);
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
            "paired_events":true,"single_read_and_apply":true});
        assert_eq!(expected, report["conservation"]);
    }

    #[test]
    fn cancellation_and_pending_work_remain_explicit() {
        let probe = fixture(0);
        read(&probe, 1..4, 0..1);
        start_apply(&probe, 5);
        let ended = at(&probe, 11);
        probe.observe(PipelineEvent::ApplyFinished(ended));
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
        assert_eq!(1, report["initial_reads"]["completed_calls"]);
        assert_eq!(0, report["extension_reads"]["completed_calls"]);
        assert_eq!(1, report["initial_reads"]["unoverlapped_ns"]["p50_ns"]);
        assert_eq!(4, report["initial_reads"]["overlap_in_window_ns"]);
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
            "paired_events":true,"single_read_and_apply":true});
        assert_eq!(expected, report["conservation"]);
        assert_eq!(2, report["all_observed_phase_counters"]["unknown_read_entry_counts"]);
        assert_eq!(0, report["all_observed_phase_counters"]["range_or_count_mismatches"]);
    }
}
