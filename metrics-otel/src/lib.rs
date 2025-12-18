//! OpenTelemetry metrics integration for Openraft.
//!
//! This crate provides an implementation of the [`MetricsRecorder`] trait
//! from openraft, allowing applications to export Raft metrics to any
//! OTEL-compatible backend (Prometheus, Jaeger, OTLP, etc.).
//!
//! # Usage
//!
//! 1. Add this crate to your `Cargo.toml`:
//!    ```toml
//!    [dependencies]
//!    openraft = "0.10"
//!    openraft-metrics-otel = "0.1"
//!    opentelemetry = "0.30"
//!    opentelemetry_sdk = "0.30"  # For configuring exporters
//!    ```
//!
//! 2. Configure the OpenTelemetry SDK in your application:
//!    ```rust,ignore
//!    use opentelemetry_sdk::metrics::MeterProvider;
//!
//!    let meter_provider = MeterProvider::builder()
//!        .with_reader(/* your exporter */)
//!        .build();
//!
//!    opentelemetry::global::set_meter_provider(meter_provider);
//!    ```
//!
//! 3. Install the recorder in your Raft instance:
//!    ```rust,ignore
//!    use std::sync::Arc;
//!    use openraft_metrics_otel::Instruments;
//!
//!    let raft = Raft::new(...).await?;
//!    raft.set_metrics_recorder(Some(Arc::new(Instruments::new()))).await?;
//!    ```
//!
//! # Metrics
//!
//! The following metrics are exported:
//!
//! ## Histograms
//! - `openraft.log.apply.batch_size` - Distribution of log entry counts in Apply commands
//! - `openraft.storage.append.batch_size` - Distribution of log entry counts when appending
//! - `openraft.replication.batch_size` - Distribution of log entry counts in replication RPCs
//! - `openraft.write.batch_size` - Distribution of client write entries merged together
//!
//! ## Gauges
//!
//! - `openraft.term.current` - Current Raft term
//! - `openraft.log.index.last` - Index of last log entry
//! - `openraft.log.index.applied` - Index of last applied log entry
//! - `openraft.snapshot.index` - Index of last snapshot
//! - `openraft.log.index.purged` - Index of last purged log entry
//! - `openraft.server.state` - Server state (0=Learner, 1=Follower, 2=Candidate, 3=Leader)
//!
//! ## Counters
//!
//! - `openraft.operation.vote.total` - Vote operations
//! - `openraft.operation.heartbeat.total` - Heartbeat operations
//! - `openraft.operation.append.total` - Append entries operations
//!
//! [`MetricsRecorder`]: openraft::metrics::MetricsRecorder

use openraft::metrics::MetricsRecorder;
use opentelemetry::global;
use opentelemetry::metrics::Counter;
use opentelemetry::metrics::Gauge;
use opentelemetry::metrics::Histogram;
use opentelemetry::metrics::Meter;

const METER_NAME: &str = "openraft";

/// OpenTelemetry instruments for recording Raft metrics.
///
/// This struct holds references to all OTEL instruments used by openraft.
/// It implements the [`MetricsRecorder`] trait for inline metrics recording
/// during Raft operations.
///
/// # Example
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use openraft_metrics_otel::Instruments;
///
/// let recorder = Arc::new(Instruments::new());
/// raft.set_metrics_recorder(Some(recorder)).await?;
/// ```
///
/// [`MetricsRecorder`]: openraft::metrics::MetricsRecorder
#[derive(Debug, Clone)]
pub struct Instruments {
    #[allow(dead_code)]
    meter: Meter,

    // Histograms for performance metrics
    apply_batch_size: Histogram<u64>,
    storage_append_batch_size: Histogram<u64>,
    replicate_batch_size: Histogram<u64>,
    write_batch_size: Histogram<u64>,

    // Gauges for runtime state
    current_term: Gauge<u64>,
    last_log_index: Gauge<u64>,
    applied_index: Gauge<u64>,
    snapshot_index: Gauge<u64>,
    purged_index: Gauge<u64>,
    server_state: Gauge<u64>,

    // Counters for operations
    vote_total: Counter<u64>,
    heartbeat_total: Counter<u64>,
    append_total: Counter<u64>,
}

impl Instruments {
    /// Creates a new set of OpenTelemetry instruments.
    ///
    /// This uses the global meter provider configured by the application.
    /// Make sure to configure the OpenTelemetry SDK before calling this.
    pub fn new() -> Self {
        let meter = global::meter(METER_NAME);

        let apply_batch_size = meter
            .u64_histogram("openraft.log.apply.batch_size")
            .with_description("Distribution of log entry counts in Apply commands")
            .build();

        let storage_append_batch_size = meter
            .u64_histogram("openraft.storage.append.batch_size")
            .with_description("Distribution of log entry counts when appending to storage")
            .build();

        let replicate_batch_size = meter
            .u64_histogram("openraft.replication.batch_size")
            .with_description("Distribution of log entry counts in replication RPCs")
            .build();

        let write_batch_size = meter
            .u64_histogram("openraft.write.batch_size")
            .with_description("Distribution of client write entries merged together")
            .build();

        let current_term = meter.u64_gauge("openraft.term.current").with_description("Current Raft term").build();

        let last_log_index =
            meter.u64_gauge("openraft.log.index.last").with_description("Index of last log entry").build();

        let applied_index = meter
            .u64_gauge("openraft.log.index.applied")
            .with_description("Index of last applied log entry")
            .build();

        let snapshot_index =
            meter.u64_gauge("openraft.snapshot.index").with_description("Index of last snapshot").build();

        let purged_index =
            meter.u64_gauge("openraft.log.index.purged").with_description("Index of last purged log entry").build();

        let server_state = meter
            .u64_gauge("openraft.server.state")
            .with_description("Server state (0=Learner, 1=Follower, 2=Candidate, 3=Leader)")
            .build();

        let vote_total = meter
            .u64_counter("openraft.operation.vote.total")
            .with_description("Total vote requests sent or received")
            .build();

        let heartbeat_total = meter
            .u64_counter("openraft.operation.heartbeat.total")
            .with_description("Total heartbeats sent")
            .build();

        let append_total = meter
            .u64_counter("openraft.operation.append.total")
            .with_description("Total append entries operations")
            .build();

        Self {
            meter,
            apply_batch_size,
            storage_append_batch_size,
            replicate_batch_size,
            write_batch_size,
            current_term,
            last_log_index,
            applied_index,
            snapshot_index,
            purged_index,
            server_state,
            vote_total,
            heartbeat_total,
            append_total,
        }
    }
}

impl Default for Instruments {
    fn default() -> Self {
        Self::new()
    }
}

/// Implements the MetricsRecorder trait for inline metrics recording.
///
/// This allows Instruments to be installed in RaftCore for automatic
/// metrics collection during Raft operations.
impl MetricsRecorder for Instruments {
    fn record_apply_batch(&self, entry_count: u64) {
        self.apply_batch_size.record(entry_count, &[]);
    }

    fn record_append_batch(&self, entry_count: u64) {
        self.storage_append_batch_size.record(entry_count, &[]);
    }

    fn record_replicate_batch(&self, entry_count: u64) {
        self.replicate_batch_size.record(entry_count, &[]);
    }

    fn record_write_batch(&self, entry_count: u64) {
        self.write_batch_size.record(entry_count, &[]);
    }

    fn set_current_term(&self, term: u64) {
        self.current_term.record(term, &[]);
    }

    fn set_last_log_index(&self, index: u64) {
        self.last_log_index.record(index, &[]);
    }

    fn set_applied_index(&self, index: u64) {
        self.applied_index.record(index, &[]);
    }

    fn set_snapshot_index(&self, index: u64) {
        self.snapshot_index.record(index, &[]);
    }

    fn set_purged_index(&self, index: u64) {
        self.purged_index.record(index, &[]);
    }

    fn set_server_state(&self, state: u8) {
        self.server_state.record(state as u64, &[]);
    }

    fn increment_vote(&self) {
        self.vote_total.add(1, &[]);
    }

    fn increment_heartbeat(&self) {
        self.heartbeat_total.add(1, &[]);
    }

    fn increment_append(&self) {
        self.append_total.add(1, &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instruments_all_methods() {
        let instruments = Instruments::new();

        // Histograms
        instruments.record_apply_batch(10);
        instruments.record_append_batch(5);
        instruments.record_replicate_batch(8);
        instruments.record_write_batch(3);

        // Gauges
        instruments.set_current_term(42);
        instruments.set_last_log_index(100);
        instruments.set_applied_index(99);
        instruments.set_snapshot_index(50);
        instruments.set_purged_index(25);
        instruments.set_server_state(3); // Leader

        // Counters
        instruments.increment_vote();
        instruments.increment_heartbeat();
        instruments.increment_append();
    }

    #[test]
    fn test_instruments_default() {
        let instruments = Instruments::default();
        instruments.record_apply_batch(1);
    }

    #[test]
    fn test_instruments_clone() {
        let instruments = Instruments::new();
        let cloned = instruments.clone();
        cloned.record_apply_batch(1);
    }
}
