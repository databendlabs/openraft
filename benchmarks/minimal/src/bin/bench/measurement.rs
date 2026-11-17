use std::time::Duration;
use std::time::Instant;

use anyhow::ensure;
use serde_json::Value;

const DEFAULT_WARMUP_SECONDS: f64 = 5.0;
const DEFAULT_DRAIN_SECONDS: f64 = 5.0;

/// Optional fixed measurement window, following warmup with the same clients and workload.
#[derive(Clone, Debug, clap::Args, serde::Serialize)]
pub(super) struct MeasurementConfig {
    /// Measure for this many seconds instead of running a fixed operation count.
    #[arg(long)]
    pub(super) measurement_seconds: Option<f64>,
    #[arg(long, default_value_t = DEFAULT_WARMUP_SECONDS)]
    pub(super) warmup_seconds: f64,
    /// Stop waiting for outstanding requests this many seconds after measurement ends.
    #[arg(long, default_value_t = DEFAULT_DRAIN_SECONDS)]
    pub(super) drain_seconds: f64,
    /// Maximum retained raw phase traces, shared across clients and operation classes.
    #[arg(long, default_value_t = 4096)]
    pub(super) trace_limit: usize,
    /// Record structured apply batches, backlogs, and source-to-publication delays.
    #[arg(long)]
    pub(super) measure_apply: bool,
}

impl MeasurementConfig {
    pub(super) fn validate(&self, limit: Duration) -> anyhow::Result<()> {
        let Some(measurement) = self.measurement_seconds else {
            return Ok(());
        };
        let durations = [self.warmup_seconds, measurement, self.drain_seconds];
        for duration in durations {
            ensure!(
                duration.is_finite() && duration >= 0.0,
                "window durations must be finite and nonnegative"
            );
        }
        ensure!(measurement > 0.0, "measurement-seconds must be positive");
        let total: f64 = durations.into_iter().sum();
        ensure!(
            total < limit.as_secs_f64(),
            "measurement window and drain exceed benchmark timeout"
        );
        let duration = Duration::from_secs_f64(measurement);
        ensure!(!duration.is_zero(), "measurement-seconds rounds to zero nanoseconds");
        let bounds = self.bounds();
        ensure!(
            bounds.deadline.unwrap() < limit,
            "converted measurement window and drain exceed benchmark timeout"
        );
        Ok(())
    }

    pub(super) fn bounds(&self) -> Bounds {
        match self.measurement_seconds {
            Some(seconds) => {
                let start = Duration::from_secs_f64(self.warmup_seconds);
                let duration = Duration::from_secs_f64(seconds);
                let end = start + duration;
                Bounds {
                    start,
                    end: Some(end),
                    deadline: Some(end + Duration::from_secs_f64(self.drain_seconds)),
                }
            }
            None => Bounds {
                start: Duration::ZERO,
                end: None,
                deadline: None,
            },
        }
    }

    pub(super) fn diagnostics(&self) -> bool {
        self.measure_apply || self.measurement_seconds.is_some()
    }

    pub(super) fn trace_budget(&self, clients: usize) -> usize {
        self.trace_limit / clients.max(1)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Bounds {
    pub(super) start: Duration,
    pub(super) end: Option<Duration>,
    pub(super) deadline: Option<Duration>,
}

impl Bounds {
    pub(super) fn contains(&self, nanos: u64) -> bool {
        let at = Duration::from_nanos(nanos);
        at >= self.start && self.end.is_none_or(|end| at < end)
    }

    pub(super) fn generating(&self, epoch: Instant, scheduled: Instant) -> bool {
        let Some(end) = self.end else {
            return true;
        };
        let end = epoch + end;
        scheduled < end && Instant::now() < end
    }

    pub(super) fn seconds(&self, elapsed: Duration) -> f64 {
        match self.end {
            Some(end) => (end - self.start).as_secs_f64(),
            None => elapsed.as_secs_f64(),
        }
    }

    pub(super) fn deadline(&self, epoch: Instant, scheduled: Instant, timeout_ms: u64) -> Deadline {
        let drain = self.deadline.map(|end| epoch + end);
        let request = (timeout_ms > 0).then(|| scheduled + Duration::from_millis(timeout_ms));
        match (request, drain) {
            (Some(request), Some(drain)) if drain < request => Deadline::Drain(drain),
            (Some(request), _) => Deadline::Request(request),
            (None, Some(drain)) => Deadline::Drain(drain),
            (None, None) => Deadline::None,
        }
    }

    pub(super) fn report(&self, elapsed: Duration) -> Value {
        let measured = self.seconds(elapsed);
        let drain = self.end.map_or(0.0, |end| elapsed.saturating_sub(end).as_secs_f64());
        serde_json::json!({"warmup_seconds":self.start.as_secs_f64(),"measurement_seconds":measured,
            "measurement_start_ns":self.start.as_nanos(),"measurement_end_ns":self.end.map(|end|end.as_nanos()),
            "drain_seconds":drain,"drain_deadline_ns":self.deadline.map(|at|at.as_nanos()),
            "latency_cohort":"scheduled_during_measurement","throughput_population":"completed_during_measurement"})
    }
}

#[derive(Clone, Copy)]
pub(super) enum Deadline {
    None,
    Request(Instant),
    Drain(Instant),
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Bounds;

    #[test]
    fn the_measurement_window_is_half_open() {
        let bounds = Bounds {
            start: Duration::from_nanos(10),
            end: Some(Duration::from_nanos(20)),
            deadline: Some(Duration::from_nanos(30)),
        };
        let actual: Vec<_> = [9, 10, 19, 20, 30].map(|at| bounds.contains(at)).into();
        assert_eq!(vec![false, true, true, false, false], actual);
        let seconds = bounds.seconds(Duration::from_secs(100));
        assert_eq!(0.00000001, seconds);
    }
}
