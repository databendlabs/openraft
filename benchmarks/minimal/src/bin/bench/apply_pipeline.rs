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

const MAX_RETAINED_SPANS: usize = 4096;

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
    pending: VecDeque<Buffered>,
    unassigned: Buffered,
    closed_reads: VecDeque<FinishedRead>,
    apply_spans: VecDeque<Range<Instant>>,
    reader_watermark: Option<Instant>,
    applier_watermark: Option<Instant>,
    retention_limit: usize,
    peak_entries: u64,
    peak_payload_bytes: u64,
    reads: ReadStats,
    apply_duration: Histogram,
    reads_per_apply: Histogram,
    applies_with_multiple_reads: u64,
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
    pending_metadata_overflow: u64,
    overlap_metadata_overflow: u64,
    timestamp_regressions: u64,
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
    overlapped_ns: Option<u64>,
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
            state: Mutex::new(State {
                retention_limit: MAX_RETAINED_SPANS,
                ..State::default()
            }),
        }
    }

    fn observe(&self, event: PipelineEvent) {
        let mut state = self.state.lock().unwrap();
        match event {
            PipelineEvent::ReadStarted { at, range } => state.start_read(at, range),
            PipelineEvent::ReadFinished { .. } => state.finish_read(event, self),
            PipelineEvent::ApplyStarted(at) => state.start_apply(at),
            PipelineEvent::Applied { at, index, entries } => state.published(at, index, entries),
            PipelineEvent::ApplyFinished { .. } => state.finish_apply(event, self),
        }
        state.reconcile(self);
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
        let coverage = state.overlap_coverage(self);
        let mut reads = state.reads.report();
        if coverage["complete"] != true {
            reads["unoverlapped_ns"] = Value::Null;
            reads["overlap_in_window_ns"] = Value::Null;
            reads["unoverlapped_time_in_window_ns"] = Value::Null;
        }
        let attributable = state.counters.pending_metadata_overflow == 0;
        let unassigned = if attributable {
            serde_json::json!(state.unassigned)
        } else {
            Value::Null
        };
        serde_json::json!({"schema_version":2,"all_reads":reads,
            "apply_duration_ns":state.apply_duration.latency(),"reads_per_apply":state.reads_per_apply.report(),
            "applies_with_multiple_reads":state.applies_with_multiple_reads,
            "conservation":state.conservation(),"all_observed_phase_counters":state.counters,
            "source_reads_unattributed":unassigned,"inflight":state.inflight(self),"buffers":state.buffers(),
            "overlap_coverage":coverage,
            "attribution_complete":attributable,
            "traces":state.traces,"trace_limit":self.trace_limit,"traces_dropped":state.traces_dropped,
            "trace_spans_seen":state.traces.len() as u64 + state.traces_dropped,
            "trace_budget_scope":"additional_global_trace_limit_budget_divided_across_nodes",
            "trace_population":"bounded_recent_finalized_spans_ending_in_measurement","histogram_relative_width_max":"1/64",
            "span_population":"store_calls_ending_in_measurement_including_straddling_calls",
            "time_population":"closed_read_spans_including_failures_and_cancellation_clipped_to_measurement; unfinished_read_time_is_excluded",
            "histogram_outcomes":"successful_calls_only; failures_and_unfinished_spans_remain_in_counters",
            "multiple_read_definition":"distinct_source_read_chunks_consumed_by_one_apply; no_channel_delivery_or_tail_extension_is_inferred",
            "assignment_validation":"actual_consumed_FIFO_prefix_at_apply_exit; unread_suffix_is_retained",
            "read_endpoint":"entries_stream_has_materialized_its_result; worker_stream_collection_is_excluded",
            "apply_endpoint":"benchmark_apply_method_exits; successful_publication_precedes_exit",
            "apply_trace_entries_scope":"stream_items_consumed_by_call; publication_is_validated_separately",
            "gap_scope":"source_materialization_to_store_apply_includes_collection_channels_and_batching; unpolled_physical_IO_completion_is_unobserved",
            "diagnostic_scope":"opt_in_store_observers_can_change_scheduling; performance_uses_feature_off"})
    }
}

impl State {
    fn start_read(&mut self, at: Instant, range: Option<Range<u64>>) {
        self.reader_watermark = Self::advance(self.reader_watermark, at, &mut self.counters.timestamp_regressions);
        self.counters.read_started += 1;
        if self.reading.is_some() {
            self.counters.overlapping_reads += 1;
        }
        if range.is_none() {
            self.counters.unknown_read_ranges += 1;
        }
        self.reading = Some(Reading { started: at, range });
    }

    fn finish_read(&mut self, event: PipelineEvent, probe: &Pipeline) {
        let PipelineEvent::ReadFinished { at, entries, status } = event else {
            unreachable!()
        };
        self.reader_watermark = Self::advance(self.reader_watermark, at, &mut self.counters.timestamp_regressions);
        let Some(read) = self.reading.take() else {
            self.counters.unpaired_events += 1;
            return;
        };
        let finished = FinishedRead {
            read,
            at,
            entries,
            status,
        };
        self.record_read(finished, probe);
    }

    fn record_read(&mut self, read: FinishedRead, probe: &Pipeline) {
        self.count_read(read.status, read.entries);
        self.buffer_read(&read.read, read.entries);
        self.reads.closed(probe, &read);
        if self.closed_reads.len() == self.retention_limit {
            self.counters.overlap_metadata_overflow += 1;
            self.trace_read(read, None, probe);
        } else {
            self.closed_reads.push_back(read);
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
        if let (Some(end), Some(range)) = (self.unassigned.end, &read.range)
            && self.unassigned.reads > 0
            && self.counters.pending_metadata_overflow == 0
            && range.start != end
        {
            self.counters.range_or_count_mismatches += 1;
        }
        let mut chunk = Buffered::default();
        chunk.include(read, counts);
        self.unassigned.extend(&chunk);
        self.peak_entries = self.peak_entries.max(self.unassigned.entries);
        self.peak_payload_bytes = self.peak_payload_bytes.max(self.unassigned.payload_bytes);
        if self.pending.len() == self.retention_limit {
            self.counters.pending_metadata_overflow += 1;
        } else {
            self.pending.push_back(chunk);
        }
    }

    fn start_apply(&mut self, at: Instant) {
        self.applier_watermark = Self::advance(self.applier_watermark, at, &mut self.counters.timestamp_regressions);
        self.counters.apply_started += 1;
        if self.applying.as_ref().is_some_and(|apply| apply.ended.is_none()) {
            self.counters.overlapping_applies += 1;
        }
        self.applying = Some(Applying {
            started: at,
            ended: None,
            buffered: Buffered::default(),
            published: None,
        });
    }

    fn published(&mut self, at: Instant, index: u64, entries: u64) {
        self.applier_watermark = Self::advance(self.applier_watermark, at, &mut self.counters.timestamp_regressions);
        let Some(apply) = self.applying.as_mut() else {
            self.counters.unpaired_events += 1;
            return;
        };
        if apply.ended.is_some() || apply.published.is_some() {
            self.counters.unpaired_events += 1;
        }
        apply.published = Some((index, entries));
    }

    fn finish_apply(&mut self, event: PipelineEvent, probe: &Pipeline) {
        let PipelineEvent::ApplyFinished {
            at,
            range,
            entries,
            payload_bytes,
            contiguous,
        } = event
        else {
            unreachable!()
        };
        self.applier_watermark = Self::advance(self.applier_watermark, at, &mut self.counters.timestamp_regressions);
        let Some(mut apply) = self.applying.take() else {
            self.counters.unpaired_events += 1;
            return;
        };
        apply.ended = Some(at);
        let consumed = Buffered::consumed(range, entries, payload_bytes);
        apply.buffered = self.consume(consumed, contiguous);
        if self.apply_spans.len() == self.retention_limit {
            self.counters.overlap_metadata_overflow += 1;
        } else {
            self.apply_spans.push_back(apply.started..at);
        }
        self.count_apply(&apply);
        if probe.contains(at) {
            if apply.published.is_some() {
                self.apply_duration.record(nanos(&(apply.started..at)));
                self.reads_per_apply.record(apply.buffered.reads);
                self.applies_with_multiple_reads += u64::from(apply.buffered.reads > 1);
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
        let active_apply = u64::from(self.applying.as_ref().is_some_and(|apply| apply.ended.is_none()));
        let reads = self.counters.read_completed + self.counters.read_failed + self.counters.read_cancelled;
        let applies = self.counters.apply_completed + self.counters.apply_ended_without_success;
        let assigned = self.counters.applied_entries + self.counters.entries_assigned_to_unsuccessful_applies;
        let entries = assigned + self.unassigned.entries;
        serde_json::json!({"read_calls":self.counters.read_started == reads + active_read,
            "apply_calls":self.counters.apply_started == applies + active_apply,
            "read_entry_assignments":self.counters.read_entries == entries,
            "range_and_count_validation":self.counters.range_or_count_mismatches == 0,
            "known_read_entry_counts":self.counters.unknown_read_entry_counts == 0,
            "known_read_ranges":self.counters.unknown_read_ranges == 0,
            "paired_events":self.counters.unpaired_events == 0,
            "single_read_and_apply":self.counters.overlapping_reads == 0 && self.counters.overlapping_applies == 0,
            "metadata_complete":self.counters.pending_metadata_overflow == 0 && self.counters.overlap_metadata_overflow == 0,
            "source_clocks_monotonic":self.counters.timestamp_regressions == 0})
    }

    fn inflight(&self, probe: &Pipeline) -> Value {
        let applying = self.applying.as_ref().filter(|apply| apply.ended.is_none());
        let started = self.reading.as_ref().map(|read| {
            let started = read.started;
            probe.offset(started)
        });
        serde_json::json!({"read":self.reading.as_ref().map(|read| &read.range),
            "read_calls":usize::from(self.reading.is_some()),"apply_calls":usize::from(applying.is_some()),
            "read_started_ns":started,"apply_started_ns":applying.map(|apply| probe.offset(apply.started)),
            "apply_consumption_scope":"known_only_at_call_exit"})
    }

    fn buffers(&self) -> Value {
        let entry_bytes = size_of::<EntryOf<TypeConfig>>();
        let traces = self.traces.capacity() * size_of::<Trace>();
        let read_histograms = self.reads.bytes();
        let histograms = read_histograms + self.apply_duration.bytes() + self.reads_per_apply.bytes();
        let metadata = self.pending.capacity() * size_of::<Buffered>()
            + self.closed_reads.capacity() * size_of::<FinishedRead>()
            + self.apply_spans.capacity() * size_of::<Range<Instant>>();
        let attributable = self.counters.pending_metadata_overflow == 0;
        let entries = if attributable { Some(self.peak_entries) } else { None };
        let payload = if attributable {
            Some(self.peak_payload_bytes)
        } else {
            None
        };
        let structs = entries.map(|count| count * entry_bytes as u64);
        serde_json::json!({"observed_peak_source_unattributed_entries":entries,
            "source_unattributed_entry_struct_bytes_estimate":structs,
            "observed_peak_source_unattributed_referenced_payload_bytes":payload,
            "peak_scope":"independent_maxima_over_all_observed_phases_including_warmup_and_drain",
            "byte_scope":"source_materialized_entries_not_yet_range_attributed; may_include_active_apply; excludes_channel_delivery_and_entry_ownership",
            "observer_histogram_bytes":histograms,"observer_trace_buffer_bytes":traces,"observer_pending_metadata_bytes":metadata})
    }

    fn advance(previous: Option<Instant>, at: Instant, regressions: &mut u64) -> Option<Instant> {
        let previous = previous.unwrap_or(at);
        if at < previous {
            *regressions += 1;
        }
        Some(previous.max(at))
    }

    fn consume(&mut self, consumed: Buffered, contiguous: bool) -> Buffered {
        if !contiguous {
            self.counters.range_or_count_mismatches += 1;
            return consumed;
        }
        if self.counters.pending_metadata_overflow > 0 {
            return consumed;
        }
        let assigned = self.assigned_prefix(&consumed);
        let Some(assigned) = assigned else {
            self.counters.range_or_count_mismatches += 1;
            return consumed;
        };
        let removed = self.remove_prefix(&assigned);
        self.unassigned.entries -= assigned.entries;
        self.unassigned.payload_bytes -= assigned.payload_bytes;
        self.unassigned.reads -= removed;
        self.unassigned.first = self.pending.front().and_then(|chunk| chunk.first);
        self.unassigned.end = self.pending.back().and_then(|chunk| chunk.end);
        assigned
    }

    fn assigned_prefix(&self, consumed: &Buffered) -> Option<Buffered> {
        if consumed.entries == 0 {
            return Some(Buffered::default());
        }
        let end = consumed.end?;
        let first = consumed.first?;
        let mut assigned = Buffered::consumed(Some(first..first), 0, 0);
        for chunk in &self.pending {
            if assigned.end == Some(end) {
                break;
            }
            if chunk.first != assigned.end {
                return None;
            }
            let remaining = consumed.payload_bytes.checked_sub(assigned.payload_bytes)?;
            let part = chunk.prefix(end, remaining)?;
            assigned.extend(&part);
        }
        let matches = assigned.end == consumed.end
            && assigned.entries == consumed.entries
            && assigned.payload_bytes == consumed.payload_bytes;
        if matches { Some(assigned) } else { None }
    }

    fn remove_prefix(&mut self, assigned: &Buffered) -> u64 {
        let mut entries = assigned.entries;
        let mut bytes = assigned.payload_bytes;
        let mut removed = 0;
        while entries > 0 {
            let chunk = self.pending.front_mut().expect("validated source prefix");
            if entries >= chunk.entries {
                entries -= chunk.entries;
                bytes -= chunk.payload_bytes;
                self.pending.pop_front();
                removed += 1;
            } else {
                let first = chunk.first.expect("validated source start");
                chunk.first = Some(first + entries);
                chunk.entries -= entries;
                chunk.payload_bytes -= bytes;
                break;
            }
        }
        removed
    }

    fn reconcile(&mut self, probe: &Pipeline) {
        while let Some(read) = self.closed_reads.front() {
            let ready = self.applier_watermark.is_some_and(|at| at >= read.at);
            if !ready {
                break;
            }
            let read = self.closed_reads.pop_front().expect("a closed read exists");
            let span = read.read.started..read.at;
            let overlap = self.overlap(span.clone());
            let clipped = probe.clipped(span);
            let in_window = clipped.map_or(0, |span| self.overlap(span));
            self.reads.resolved(probe, &read, overlap, in_window);
            self.trace_read(read, Some(overlap), probe);
        }
        let closed = self.closed_reads.front().map(|read| read.read.started);
        let active = self.reading.as_ref().map(|read| read.started);
        let before = closed.or(active).or(self.reader_watermark);
        let Some(before) = before else { return };
        while self.apply_spans.front().is_some_and(|span| span.end <= before) {
            self.apply_spans.pop_front();
        }
    }

    fn overlap(&self, span: Range<Instant>) -> u64 {
        let mut total = 0;
        for applying in &self.apply_spans {
            total += intersection(&span, applying);
        }
        if let Some(applying) = self.applying.as_ref().filter(|apply| apply.ended.is_none())
            && let Some(through) = self.applier_watermark
        {
            total += intersection(&span, &(applying.started..through));
        }
        total
    }

    fn trace_read(&mut self, read: FinishedRead, overlap: Option<u64>, probe: &Pipeline) {
        if probe.contains(read.at) {
            let trace = Trace::read(probe, &read, overlap);
            self.trace(trace, probe.trace_limit);
        }
    }

    fn overlap_coverage(&self, probe: &Pipeline) -> Value {
        let unresolved = self
            .closed_reads
            .iter()
            .filter(|read| {
                let span = read.read.started..read.at;
                probe.contains(read.at) || probe.clipped(span).is_some_and(|clipped| nanos(&clipped) > 0)
            })
            .count();
        let complete =
            unresolved == 0 && self.counters.overlap_metadata_overflow == 0 && self.counters.timestamp_regressions == 0;
        serde_json::json!({"complete":complete,"unresolved_closed_reads":self.closed_reads.len(),
            "unresolved_in_window":unresolved,"retained_apply_spans":self.apply_spans.len(),"metadata_limit":self.retention_limit,
            "overflow":self.counters.overlap_metadata_overflow,"timestamp_scope":"original_source_timestamps; independent_serial_stage_watermarks"})
    }
}

impl ReadStats {
    fn closed(&mut self, probe: &Pipeline, read: &FinishedRead) {
        let span = read.read.started..read.at;
        if let Some(clipped) = probe.clipped(span.clone()) {
            self.time_in_window_ns += nanos(&clipped);
        }
        if let Some((entries, _)) = read.entries
            && probe.contains(span.end)
        {
            let total = nanos(&span);
            self.duration.record(total);
            self.entries.record(entries);
        }
    }

    fn resolved(&mut self, probe: &Pipeline, read: &FinishedRead, overlap: u64, in_window: u64) {
        self.overlap_in_window_ns += in_window;
        if read.entries.is_some() && probe.contains(read.at) {
            let total = nanos(&(read.read.started..read.at));
            self.unoverlapped.record(total.saturating_sub(overlap));
        }
    }

    fn report(&self) -> Value {
        serde_json::json!({"completed_calls":self.duration.count,"duration_ns":self.duration.latency(),
            "unoverlapped_ns":self.unoverlapped.latency(),"overlap_resolved_calls":self.unoverlapped.count,"entries":self.entries.report(),
            "time_in_window_ns":self.time_in_window_ns,"overlap_in_window_ns":self.overlap_in_window_ns,
            "unoverlapped_time_in_window_ns":self.time_in_window_ns.saturating_sub(self.overlap_in_window_ns)})
    }

    fn bytes(&self) -> usize {
        self.duration.bytes() + self.unoverlapped.bytes() + self.entries.bytes()
    }
}

impl Trace {
    fn read(probe: &Pipeline, read: &FinishedRead, overlap: Option<u64>) -> Self {
        Self {
            kind: "read",
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
        let counts = Some((apply.buffered.entries, apply.buffered.payload_bytes));
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
            overlapped_ns: None,
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
        if entries == 0 {
            return self.entries == 0;
        }
        let count_matches = entries == self.entries;
        let range_matches = match (self.first, self.end) {
            (Some(first), Some(end)) => first.checked_add(entries) == Some(end) && index.checked_add(1) == Some(end),
            _ => true,
        };
        count_matches && range_matches
    }

    fn consumed(range: Option<Range<u64>>, entries: u64, payload_bytes: u64) -> Self {
        let first = range.as_ref().map(|range| range.start);
        let end = range.as_ref().map(|range| range.end);
        Self {
            first,
            end,
            reads: 0,
            entries,
            payload_bytes,
            unknown_reads: 0,
        }
    }

    fn prefix(&self, end: u64, remaining_payload: u64) -> Option<Self> {
        let first = self.first?;
        let source_end = self.end?;
        let count = source_end.checked_sub(first)?;
        if self.unknown_reads > 0 || self.entries != count {
            return None;
        }
        let end = end.min(source_end);
        let entries = end.checked_sub(first)?;
        let payload = if end == source_end {
            self.payload_bytes
        } else {
            remaining_payload
        };
        if payload > self.payload_bytes || entries == 0 {
            return None;
        }
        Some(Self {
            first: Some(first),
            end: Some(end),
            reads: 1,
            entries,
            payload_bytes: payload,
            unknown_reads: 0,
        })
    }

    fn extend(&mut self, other: &Self) {
        if self.reads == 0 {
            self.first = other.first;
        }
        self.end = other.end;
        self.reads += other.reads;
        self.entries += other.entries;
        self.payload_bytes += other.payload_bytes;
        self.unknown_reads += other.unknown_reads;
    }
}

fn intersection(left: &Range<Instant>, right: &Range<Instant>) -> u64 {
    let start = left.start.max(right.start);
    let end = left.end.min(right.end);
    nanos(&(start..end))
}

fn nanos(range: &Range<Instant>) -> u64 {
    range.end.saturating_duration_since(range.start).as_nanos() as u64
}

#[cfg(test)]
#[path = "apply_pipeline/apply_pipeline_test.rs"]
mod apply_pipeline_test;
