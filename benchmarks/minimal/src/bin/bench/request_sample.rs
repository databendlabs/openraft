use std::time::Duration;
use std::time::Instant;

use super::arrival::Arrival;
use super::measurement::Deadline;
pub(super) use super::summary::Samples;
use super::write_progress::WriteProgress;

pub(super) struct LegacyReadSample {
    pub(super) latency_ns: u64,
    pub(super) status: Result<(), String>,
}

/// One offered request, including requests rejected before dispatch.
pub(super) struct RequestSample {
    pub(super) scheduled_ns: u64,
    pub(super) invoked_ns: Option<u64>,
    pub(super) invoked: bool,
    pub(super) latency_ns: u64,
    pub(super) node: Option<u64>,
    pub(super) outcome: Outcome,
    pub(super) retries: u64,
    pub(super) phases: Option<Box<PhaseTrace>>,
    pub(super) write_progress: Option<Box<WriteProgress>>,
}

#[derive(Debug, PartialEq)]
pub(super) enum Outcome {
    Completed,
    Failed(String),
    TimedOut,
    Rejected,
    Pending,
    Missed,
}

impl Outcome {
    pub(super) fn index(&self) -> usize {
        match self {
            Self::Completed => 0,
            Self::Failed(_) => 1,
            Self::TimedOut => 2,
            Self::Rejected => 3,
            Self::Pending => 4,
            Self::Missed => 5,
        }
    }
}

/// Sampled phase boundaries, relative to the workload epoch.
#[derive(Default, serde::Serialize)]
pub(super) struct PhaseTrace {
    pub(super) operation: u64,
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
                operation: arrival.operation,
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
            invoked: false,
            latency_ns: 0,
            node: None,
            outcome: Outcome::Completed,
            retries: 0,
            phases,
            write_progress: None,
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

#[derive(Debug, PartialEq)]
pub(super) enum Completion<T> {
    Completed(T),
    TimedOut,
    Pending,
}

/// A request deadline includes client backlog and retries; drain expiry preserves uncertainty.
pub(super) async fn with_deadline<F, T>(future: F, deadline: Deadline) -> anyhow::Result<Completion<T>>
where F: std::future::Future<Output = anyhow::Result<T>> {
    let at = match deadline {
        Deadline::None => return future.await.map(Completion::Completed),
        Deadline::Request(at) | Deadline::Drain(at) => at,
    };
    let expired = match deadline {
        Deadline::Drain(_) => Completion::Pending,
        _ => Completion::TimedOut,
    };
    if at <= Instant::now() {
        return Ok(expired);
    }
    match tokio::time::timeout_at(tokio::time::Instant::from_std(at), future).await {
        Ok(result) => result.map(Completion::Completed),
        Err(_) => Ok(expired),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use std::time::Instant;

    use super::super::measurement::Bounds;
    use super::Completion;
    use super::Deadline;
    use super::Outcome;
    use super::PhaseTrace;
    use super::RequestSample;
    use super::Samples;
    use super::with_deadline;

    #[tokio::test]
    async fn an_expired_queued_request_never_invokes_the_service() -> anyhow::Result<()> {
        let invoked = AtomicBool::new(false);
        let future = async {
            invoked.store(true, Ordering::Relaxed);
            Ok(())
        };
        let scheduled = Instant::now() - Duration::from_secs(1);
        let deadline = Deadline::Request(scheduled + Duration::from_millis(10));
        let result = with_deadline(future, deadline).await?;
        assert_eq!(Completion::TimedOut, result);
        let actual = invoked.load(Ordering::Relaxed);
        assert!(!actual);
        Ok(())
    }

    #[test]
    fn phase_intervals_and_outcomes_are_reported_separately() -> anyhow::Result<()> {
        let sample = |outcome, phases| RequestSample {
            scheduled_ns: 10,
            invoked_ns: None,
            invoked: false,
            latency_ns: 90,
            node: None,
            outcome,
            retries: 2,
            phases,
            write_progress: None,
        };
        let phases = Box::new(PhaseTrace {
            operation: 0,
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
        let mut summary = Samples::new(
            Bounds {
                start: Duration::ZERO,
                end: None,
                deadline: None,
            },
            4,
        );
        for sample in samples {
            summary.record(sample);
        }
        let actual = summary.timing();
        let requests = serde_json::json!({"completed":1,"failed":1,"timed_out":1,"rejected":1,"pending":0,"missed":0});
        let expected_phases = serde_json::json!({ "samples": 1,"traces_dropped":0,
            "client_delay": {"samples": 1, "mean_ns": 20, "p50_ns": 20, "p95_ns": 20, "p99_ns": 20},
            "service": {"samples": 1, "mean_ns": 70, "p50_ns": 70, "p95_ns": 70, "p99_ns": 70},
            "queue_wait": {"samples": 1, "mean_ns": 10, "p50_ns": 10, "p95_ns": 10, "p99_ns": 10},
            "generator_delay": {"samples": 1, "mean_ns": 10, "p50_ns": 10, "p95_ns": 10, "p99_ns": 10},
            "authorization": {"samples": 1, "mean_ns": 10, "p50_ns": 10, "p95_ns": 10, "p99_ns": 10},
            "barrier_wait": {"samples": 1, "mean_ns": 30, "p50_ns": 30, "p95_ns": 30, "p99_ns": 30},
            "query_lock_wait": {"samples": 1, "mean_ns": 20, "p50_ns": 20, "p95_ns": 20, "p99_ns": 20},
            "query": {"samples": 1, "mean_ns": 10, "p50_ns": 10, "p95_ns": 10, "p99_ns": 10},
            "traces": [{"operation":0, "scheduled_ns":10, "released_ns":20, "dispatched_ns":30,
                "authorized_ns":40, "barrier_ready_ns":70, "query_lock_ns":90, "completed_ns":100}]
        });
        let expected = serde_json::json!({ "offered": 4, "admitted": 3, "completed": 1,
            "failed": 1, "timed_out": 1, "rejected": 1, "pending":0,"missed":0,"all_phase_outcomes":{"completed":1,"failed":1,"timed_out":1,"rejected":1,"pending":0,"missed":0},
            "unanswered_invoked":0,"unanswered_not_invoked":1,
            "request_outcomes":requests,"all_phase_request_outcomes":requests,
            "operations_per_request":1,"outcome_unit":"operation","latency_unit":"request_or_batch","errors_unit":"request_or_batch",
            "retries": 8, "errors": {"failure": 1},"errors_omitted":0,"warmup_requests":0,
            "phases": expected_phases });
        assert_eq!(expected, actual);
        Ok(())
    }
    #[tokio::test]
    async fn drain_expiry_preserves_an_unanswered_request() -> anyhow::Result<()> {
        let invoked = AtomicBool::new(false);
        let future = async {
            invoked.store(true, Ordering::Relaxed);
            Ok(())
        };
        let deadline = Deadline::Drain(Instant::now() - Duration::from_secs(1));
        let result = with_deadline(future, deadline).await?;
        assert_eq!(Completion::Pending, result);
        let actual = invoked.load(Ordering::Relaxed);
        assert!(!actual);
        Ok(())
    }
}
