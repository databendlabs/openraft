use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use bench_minimal::network::BenchRaft;
use bench_minimal::store::TypeConfig;
use openraft::LogIdOptionExt;
use openraft::alias::EntryOf;
use openraft::async_runtime::WatchReceiver;
use openraft::metrics::MetricsRecorder;
use serde_json::Value;
use tokio::sync::Notify;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::histogram::Histogram;
use super::measurement::Bounds;

const SOURCE_CAPACITY: usize = 4096;
const BACKLOG_CAPACITY: usize = 4096;
const BACKLOG_INTERVAL: Duration = Duration::from_millis(100);

/// Benchmark-only apply observations. Every queue and histogram has a fixed memory bound.
#[derive(Debug)]
pub(super) struct ApplyMetrics {
    epoch: Arc<OnceLock<Instant>>,
    bounds: Bounds,
    state: Mutex<State>,
    installed: AtomicBool,
    ready: Notify,
}

#[derive(Debug)]
struct State {
    source: VecDeque<(u64, u64)>,
    watch_source: VecDeque<(u64, u64)>,
    watch_dropped: u64,
    source_dropped: u64,
    source_coalesced: u64,
    batches: Histogram,
    published_batches: u64,
    publication: Histogram,
    watch_observed: Histogram,
    backlog: Histogram,
    series: VecDeque<(u64, u64, u64)>,
    series_dropped: u64,
    committed: u64,
    applied: u64,
    last_log: u64,
    purged: Option<u64>,
    last_sample: Option<Instant>,
    watch_coalesced: u64,
}

impl ApplyMetrics {
    pub(super) fn new(epoch: Arc<OnceLock<Instant>>, bounds: Bounds) -> Self {
        Self {
            epoch,
            bounds,
            state: Mutex::new(State {
                source: VecDeque::new(),
                watch_source: VecDeque::new(),
                watch_dropped: 0,
                source_dropped: 0,
                source_coalesced: 0,
                batches: Histogram::default(),
                published_batches: 0,
                publication: Histogram::default(),
                watch_observed: Histogram::default(),
                backlog: Histogram::default(),
                series: VecDeque::new(),
                series_dropped: 0,
                committed: 0,
                applied: 0,
                last_log: 0,
                purged: None,
                last_sample: None,
                watch_coalesced: 0,
            }),
            installed: AtomicBool::new(false),
            ready: Notify::new(),
        }
    }

    pub(super) async fn wait_installed(&self) {
        loop {
            let ready = self.ready.notified();
            if self.installed.load(Ordering::Acquire) {
                return;
            }
            ready.await;
        }
    }

    pub(super) fn source_applied(&self, index: u64, entries: u64, at: Instant) {
        let Some(epoch) = self.epoch.get() else {
            return;
        };
        let nanos = at.saturating_duration_since(*epoch).as_nanos() as u64;
        let mut state = self.state.lock().unwrap();
        if self.bounds.contains(nanos) {
            state.batches.record(entries);
        }
        if state.source.len() == SOURCE_CAPACITY {
            state.source.pop_front();
            state.source_dropped += 1;
        }
        state.source.push_back((index, nanos));
        if state.watch_source.len() == SOURCE_CAPACITY {
            state.watch_source.pop_front();
            state.watch_dropped += 1;
        }
        state.watch_source.push_back((index, nanos));
    }

    pub(super) fn watch(
        self: &Arc<Self>,
        runtime: &tokio::runtime::Handle,
        raft: BenchRaft,
    ) -> (watch::Sender<bool>, JoinHandle<anyhow::Result<()>>) {
        let metrics = self.clone();
        let (stop, mut stopped) = watch::channel(false);
        let handle = runtime.spawn(async move {
            let mut rx = raft.metrics();
            loop {
                let applied = rx.borrow_watched().last_applied.index();
                if let Some(index) = applied {
                    metrics.watch_observed(index);
                }
                tokio::select! { changed=rx.changed()=>changed?,_=stopped.changed()=>break }
            }
            Ok(())
        });
        (stop, handle)
    }

    fn watch_observed(&self, index: u64) {
        let Some(at) = self.at() else {
            return;
        };
        let mut state = self.state.lock().unwrap();
        let mut matched = 0u64;
        while state.watch_source.front().is_some_and(|(source, _)| *source <= index) {
            let (_, source) = state.watch_source.pop_front().unwrap();
            if self.bounds.contains(source) {
                state.watch_observed.record(at.saturating_sub(source));
                matched += 1;
            }
        }
        state.watch_coalesced += matched.saturating_sub(1);
    }

    fn at(&self) -> Option<u64> {
        let epoch = self.epoch.get()?;
        Some(epoch.elapsed().as_nanos() as u64)
    }

    fn sample_backlog(&self, state: &mut State, at: u64) {
        if !self.bounds.contains(at) {
            return;
        }
        state.backlog.record(state.committed.saturating_sub(state.applied));
        let now = Instant::now();
        if state.last_sample.is_some_and(|last| now.duration_since(last) < BACKLOG_INTERVAL) {
            return;
        }
        state.last_sample = Some(now);
        if state.series.len() == BACKLOG_CAPACITY {
            state.series.pop_front();
            state.series_dropped += 1;
        }
        let point = (at, state.committed, state.applied);
        state.series.push_back(point);
    }

    pub(super) fn report(&self) -> Value {
        let state = self.state.lock().unwrap();
        let retained = state.last_log.saturating_add(1).saturating_sub(state.purged.map_or(0, |index| index + 1));
        let entry_bytes = size_of::<EntryOf<TypeConfig>>();
        let histogram_bytes =
            state.batches.bytes() + state.publication.bytes() + state.watch_observed.bytes() + state.backlog.bytes();
        serde_json::json!({"apply_calls":state.batches.count,"entries_per_apply":state.batches.report(),
            "core_reported_apply_calls":state.published_batches,"source_to_recorder_callback_ns":state.publication.latency(),
            "source_to_metrics_watch_observed_ns":state.watch_observed.latency(),"watch_source_records_dropped":state.watch_dropped,
            "watch_source_records_coalesced":state.watch_coalesced,"unobserved_watch_source_records":state.watch_source.len(),
            "watch_observation_scope":"server_runtime_includes_observer_scheduling_after_watch_publication",
            "source_records_dropped":state.source_dropped,"source_records_coalesced":state.source_coalesced,
            "unpublished_source_records":state.source.len(),"commit_to_apply_entries":state.backlog.report(),
            "backlog_series":state.series,"backlog_series_dropped":state.series_dropped,
            "last_committed":state.committed,"last_applied":state.applied,
            "memory":{"histogram_bytes":histogram_bytes,"source_record_capacity":SOURCE_CAPACITY,
                "retained_log_entries":retained,"retained_entry_bytes_estimate":retained*entry_bytes as u64,
                "unapplied_entry_bytes_estimate":state.committed.saturating_sub(state.applied)*entry_bytes as u64,
                "estimate_scope":"inline_entry_metadata_excludes_tree_nodes_and_allocator_overhead",
                "worker_prefetch_bytes":null},"batch_population":"apply_calls_completed_in_measurement"})
    }
}

impl MetricsRecorder for ApplyMetrics {
    fn record_apply_batch(&self, _entry_count: u64) {
        let Some(at) = self.at() else {
            return;
        };
        if self.bounds.contains(at) {
            self.state.lock().unwrap().published_batches += 1;
        }
    }
    fn record_append_batch(&self, _entry_count: u64) {}
    fn record_write_batch(&self, _entry_count: u64) {}
    fn set_current_term(&self, _term: u64) {}
    fn set_last_log_index(&self, index: u64) {
        self.state.lock().unwrap().last_log = index;
    }
    fn set_committed_index(&self, index: u64) {
        let mut state = self.state.lock().unwrap();
        state.committed = index;
    }
    fn set_applied_index(&self, index: u64) {
        let mut state = self.state.lock().unwrap();
        state.applied = index;
        let Some(at) = self.at() else {
            return;
        };
        let mut matched = 0u64;
        while state.source.front().is_some_and(|(source, _)| *source <= index) {
            let (_, source) = state.source.pop_front().unwrap();
            if self.bounds.contains(source) {
                state.publication.record(at.saturating_sub(source));
                matched += 1;
            }
        }
        state.source_coalesced += matched.saturating_sub(1);
    }
    fn set_snapshot_index(&self, _index: u64) {}
    fn set_purged_index(&self, index: u64) {
        self.state.lock().unwrap().purged = Some(index);
    }
    fn set_server_state(&self, _state: u8) {
        if let Some(at) = self.at() {
            let mut state = self.state.lock().unwrap();
            self.sample_backlog(&mut state, at);
        }
        self.installed.store(true, Ordering::Release);
        self.ready.notify_waiters();
    }
    fn increment_vote(&self) {}
    fn increment_heartbeat(&self) {}
    fn increment_append(&self) {}
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::OnceLock;
    use std::time::Duration;
    use std::time::Instant;

    use openraft::metrics::MetricsRecorder;

    use super::ApplyMetrics;
    use super::Bounds;

    #[test]
    fn source_completion_excludes_warmup_and_waits_for_publication() {
        let epoch = Arc::new(OnceLock::new());
        let origin = Instant::now() - Duration::from_millis(10);
        epoch.set(origin).unwrap();
        let metrics = ApplyMetrics::new(epoch, Bounds {
            start: Duration::from_millis(5),
            end: Some(Duration::from_millis(20)),
            deadline: None,
        });
        metrics.source_applied(1, 1, origin + Duration::from_millis(1));
        metrics.source_applied(3, 2, origin + Duration::from_millis(6));
        let before = metrics.report();
        let expected = serde_json::json!(1);
        assert_eq!(expected, before["apply_calls"]);
        let expected = serde_json::json!(2);
        assert_eq!(expected, before["unpublished_source_records"]);
        assert_eq!(serde_json::Value::Null, before["source_to_recorder_callback_ns"]);
        metrics.set_applied_index(3);
        let after = metrics.report();
        let expected = serde_json::json!(0);
        assert_eq!(expected, after["unpublished_source_records"]);
        let expected = serde_json::json!(1);
        assert_eq!(expected, after["source_to_recorder_callback_ns"]["samples"]);
    }
    #[test]
    fn backlog_uses_one_coherent_metrics_publication() {
        let epoch = Arc::new(OnceLock::new());
        epoch.set(Instant::now()).unwrap();
        let metrics = ApplyMetrics::new(epoch, Bounds {
            start: Duration::ZERO,
            end: None,
            deadline: None,
        });
        for index in [10, 20] {
            metrics.set_committed_index(index);
            metrics.set_applied_index(index);
            metrics.set_server_state(3);
        }
        let report = metrics.report();
        let expected = serde_json::json!(0);
        assert_eq!(expected, report["commit_to_apply_entries"]["max"]);
    }

    #[test]
    fn watch_observation_counts_coalesced_source_records() {
        let epoch = Arc::new(OnceLock::new());
        let origin = Instant::now() - Duration::from_millis(10);
        epoch.set(origin).unwrap();
        let metrics = ApplyMetrics::new(epoch, Bounds {
            start: Duration::ZERO,
            end: None,
            deadline: None,
        });
        metrics.source_applied(1, 1, origin);
        metrics.source_applied(2, 1, origin);
        let before = metrics.report();
        let expected = serde_json::json!(2);
        assert_eq!(expected, before["unobserved_watch_source_records"]);
        metrics.watch_observed(2);
        let after = metrics.report();
        let expected = serde_json::json!(1);
        assert_eq!(expected, after["watch_source_records_coalesced"]);
        let expected = serde_json::json!(0);
        assert_eq!(expected, after["unobserved_watch_source_records"]);
        let expected = serde_json::json!(2);
        assert_eq!(expected, after["source_to_metrics_watch_observed_ns"]["samples"]);
    }
}
