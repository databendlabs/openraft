use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use serde_json::Value;

use super::arrival::Arrival;
use super::percentile;

/// One offered request, including requests rejected before dispatch.
pub(super) struct RequestSample {
    pub(super) scheduled_ns: u64,
    pub(super) invoked_ns: Option<u64>,
    pub(super) latency_ns: u64,
    pub(super) node: Option<u64>,
    pub(super) outcome: Outcome,
    pub(super) retries: u64,
    pub(super) phases: Option<Box<PhaseTrace>>,
}

#[derive(Debug, PartialEq)]
pub(super) enum Outcome {
    Completed,
    Failed(String),
    TimedOut,
    Rejected,
}

/// Sampled phase boundaries, relative to the workload epoch.
#[derive(Default, serde::Serialize)]
pub(super) struct PhaseTrace {
    pub(super) scheduled_ns: u64,
    pub(super) dispatched_ns: u64,
    pub(super) released_ns: Option<u64>,
    pub(super) authorized_ns: Option<u64>,
    pub(super) barrier_ready_ns: Option<u64>,
    pub(super) query_lock_ns: Option<u64>,
    pub(super) completed_ns: u64,
}

impl RequestSample {
    pub(super) fn new(epoch: Instant, arrival: Arrival, sampled: bool) -> anyhow::Result<Self> {
        let scheduled_ns = nanos(epoch, arrival.scheduled)?;
        let phases = if sampled {
            let dispatched_ns = nanos(epoch, Instant::now())?;
            let released_ns = match arrival.released {
                Some(at) => Some(nanos(epoch, at)?),
                None => None,
            };
            Some(Box::new(PhaseTrace {
                scheduled_ns,
                dispatched_ns,
                released_ns,
                ..Default::default()
            }))
        } else {
            None
        };
        Ok(Self {
            scheduled_ns,
            invoked_ns: None,
            latency_ns: 0,
            node: None,
            outcome: Outcome::Completed,
            retries: 0,
            phases,
        })
    }

    pub(super) fn finish(&mut self, epoch: Instant, scheduled: Instant) -> anyhow::Result<()> {
        let finished = Instant::now();
        self.latency_ns = duration_ns(finished.saturating_duration_since(scheduled))?;
        if let Some(phases) = &mut self.phases {
            phases.completed_ns = nanos(epoch, finished)?;
        }
        Ok(())
    }

    pub(super) fn mark_authorized(&mut self, epoch: Instant) -> anyhow::Result<()> {
        if let Some(phases) = &mut self.phases {
            phases.authorized_ns = Some(nanos(epoch, Instant::now())?);
        }
        Ok(())
    }

    pub(super) fn mark_barrier(&mut self, epoch: Instant) -> anyhow::Result<()> {
        if let Some(phases) = &mut self.phases {
            phases.barrier_ready_ns = Some(nanos(epoch, Instant::now())?);
        }
        Ok(())
    }

    pub(super) fn mark_query_lock(&mut self, epoch: Instant, locked: Instant) -> anyhow::Result<()> {
        if let Some(phases) = &mut self.phases {
            phases.query_lock_ns = Some(nanos(epoch, locked)?);
        }
        Ok(())
    }
}

pub(super) fn duration_ns(duration: Duration) -> anyhow::Result<u64> {
    let value = u64::try_from(duration.as_nanos())?;
    Ok(value)
}

pub(super) fn nanos(epoch: Instant, at: Instant) -> anyhow::Result<u64> {
    duration_ns(at.saturating_duration_since(epoch))
}

pub(super) fn latency(mut samples: Vec<u64>) -> anyhow::Result<Value> {
    if samples.is_empty() {
        return Ok(Value::Null);
    }
    samples.sort_unstable();
    let total: u128 = samples.iter().map(|value| u128::from(*value)).sum();
    let mean = total / samples.len() as u128;
    let mean = u64::try_from(mean)?;
    Ok(serde_json::json!({ "samples": samples.len(), "mean_ns": mean,
        "p50_ns": percentile(&samples, 50), "p95_ns": percentile(&samples, 95),
        "p99_ns": percentile(&samples, 99) }))
}

pub(super) fn timing_report(samples: &[RequestSample], entries: u64) -> anyhow::Result<Value> {
    let mut counts = [0u64; 4];
    for sample in samples {
        let index = match sample.outcome {
            Outcome::Completed => 0,
            Outcome::Failed(_) => 1,
            Outcome::TimedOut => 2,
            Outcome::Rejected => 3,
        };
        counts[index] += entries;
    }
    let offered = counts.iter().sum::<u64>();
    let admitted = offered - counts[3];
    let retries = samples.iter().map(|sample| sample.retries).sum::<u64>();
    let phases = phase_report(samples)?;
    Ok(serde_json::json!({ "offered": offered, "admitted": admitted,
        "completed": counts[0], "failed": counts[1], "timed_out": counts[2], "rejected": counts[3],
        "retries": retries, "phases": phases }))
}

fn phase_report(samples: &[RequestSample]) -> anyhow::Result<Value> {
    let traces: Vec<_> = samples.iter().filter_map(|sample| sample.phases.as_ref()).collect();
    let client = traces.iter().map(|trace| trace.dispatched_ns - trace.scheduled_ns).collect();
    let queue = traces.iter().filter_map(|trace| Some(trace.dispatched_ns - trace.released_ns?)).collect();
    let generator = traces.iter().filter_map(|trace| Some(trace.released_ns? - trace.scheduled_ns)).collect();
    let service = traces.iter().map(|trace| trace.completed_ns - trace.dispatched_ns).collect();
    let authorization =
        traces.iter().filter_map(|trace| trace.authorized_ns.map(|at| at - trace.dispatched_ns)).collect();
    let barrier = traces.iter().filter_map(|trace| Some(trace.barrier_ready_ns? - trace.authorized_ns?)).collect();
    let lock = traces.iter().filter_map(|trace| Some(trace.query_lock_ns? - trace.barrier_ready_ns?)).collect();
    let query = traces.iter().filter_map(|trace| Some(trace.completed_ns - trace.query_lock_ns?)).collect();
    Ok(
        serde_json::json!({ "samples": traces.len(), "client_delay": latency(client)?,
        "service": latency(service)?, "queue_wait": latency(queue)?,
        "generator_delay": latency(generator)?, "authorization": latency(authorization)?,
        "barrier_wait": latency(barrier)?, "query_lock_wait": latency(lock)?,
        "query": latency(query)?, "traces": traces }),
    )
}

pub(super) fn latency_report(samples: Vec<u64>, elapsed: Duration, entries: u64) -> anyhow::Result<Value> {
    let count = u64::try_from(samples.len())?;
    let operations = count.checked_mul(entries).context("measured operations overflows")?;
    let operations_per_second = operations as f64 / elapsed.as_secs_f64();
    let latency = latency(samples)?;
    Ok(serde_json::json!({ "operations": operations,
        "operations_per_second": operations_per_second, "latency": latency }))
}

/// A timeout includes arrival backlog and all retries of the original request.
pub(super) async fn with_deadline<F, T>(future: F, scheduled: Instant, timeout_ms: u64) -> anyhow::Result<Option<T>>
where F: std::future::Future<Output = anyhow::Result<T>> {
    if timeout_ms == 0 {
        return future.await.map(Some);
    }
    let deadline = scheduled + Duration::from_millis(timeout_ms);
    if deadline <= Instant::now() {
        return Ok(None);
    }
    let deadline = tokio::time::Instant::from_std(deadline);
    match tokio::time::timeout_at(deadline, future).await {
        Ok(result) => result.map(Some),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use std::time::Instant;

    use super::Outcome;
    use super::PhaseTrace;
    use super::RequestSample;
    use super::timing_report;
    use super::with_deadline;

    #[tokio::test]
    async fn an_expired_queued_request_never_invokes_the_service() -> anyhow::Result<()> {
        let invoked = AtomicBool::new(false);
        let future = async {
            invoked.store(true, Ordering::Relaxed);
            Ok(())
        };
        let scheduled = Instant::now() - Duration::from_secs(1);
        let result = with_deadline(future, scheduled, 10).await?;
        assert_eq!(None, result);
        let actual = invoked.load(Ordering::Relaxed);
        assert!(!actual);
        Ok(())
    }

    #[test]
    fn phase_intervals_and_outcomes_are_reported_separately() -> anyhow::Result<()> {
        let sample = |outcome, phases| RequestSample {
            scheduled_ns: 10,
            invoked_ns: None,
            latency_ns: 90,
            node: None,
            outcome,
            retries: 2,
            phases,
        };
        let phases = Box::new(PhaseTrace {
            scheduled_ns: 10,
            released_ns: Some(20),
            dispatched_ns: 30,
            authorized_ns: Some(40),
            barrier_ready_ns: Some(70),
            query_lock_ns: Some(90),
            completed_ns: 100,
        });
        let samples = vec![
            sample(Outcome::Completed, Some(phases)),
            sample(Outcome::Failed("failure".into()), None),
            sample(Outcome::TimedOut, None),
            sample(Outcome::Rejected, None),
        ];
        let actual = timing_report(&samples, 1)?;
        let expected = serde_json::json!({ "offered": 4, "admitted": 3, "completed": 1,
            "failed": 1, "timed_out": 1, "rejected": 1, "retries": 8,
            "phases": { "samples": 1,
                "client_delay": {"samples": 1, "mean_ns": 20, "p50_ns": 20, "p95_ns": 20, "p99_ns": 20},
                "service": {"samples": 1, "mean_ns": 70, "p50_ns": 70, "p95_ns": 70, "p99_ns": 70},
                "queue_wait": {"samples": 1, "mean_ns": 10, "p50_ns": 10, "p95_ns": 10, "p99_ns": 10},
                "generator_delay": {"samples": 1, "mean_ns": 10, "p50_ns": 10, "p95_ns": 10, "p99_ns": 10},
                "authorization": {"samples": 1, "mean_ns": 10, "p50_ns": 10, "p95_ns": 10, "p99_ns": 10},
                "barrier_wait": {"samples": 1, "mean_ns": 30, "p50_ns": 30, "p95_ns": 30, "p99_ns": 30},
                "query_lock_wait": {"samples": 1, "mean_ns": 20, "p50_ns": 20, "p95_ns": 20, "p99_ns": 20},
                "query": {"samples": 1, "mean_ns": 10, "p50_ns": 10, "p95_ns": 10, "p99_ns": 10},
                "traces": [{"scheduled_ns":10, "released_ns":20, "dispatched_ns":30,
                    "authorized_ns":40, "barrier_ready_ns":70, "query_lock_ns":90, "completed_ns":100}]
            } });
        assert_eq!(expected, actual);
        Ok(())
    }
}
