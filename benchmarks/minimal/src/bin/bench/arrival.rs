use std::future::Future;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use tokio::sync::Barrier;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::measurement::MeasurementConfig;
use super::request_sample::Outcome;
use super::request_sample::RequestSample;
use super::request_sample::Samples;
use super::run_monitor::Live;

const YIELD_BATCH: u64 = 64;
pub(super) const READ_STREAM: u64 = 0x72656164;
pub(super) const WRITE_STREAM: u64 = 0x7772697465;

#[derive(Clone, Copy, Debug, Default, PartialEq, clap::ValueEnum, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Pattern {
    #[default]
    Uniform,
    Poisson,
    Burst,
}

/// Shared controls for both read and write arrival streams.
#[derive(Clone, Debug, clap::Args, serde::Serialize)]
pub(super) struct ArrivalConfig {
    /// Generate arrivals independently of request completion, with bounded admission.
    #[arg(long)]
    pub(super) open_loop: bool,
    /// Arrival spacing; Poisson is reproducible with --arrival-seed.
    #[arg(long, value_enum, default_value = "uniform")]
    pub(super) arrival_pattern: Pattern,
    #[arg(long, default_value_t = 1)]
    pub(super) arrival_seed: u64,
    /// Number of simultaneous arrivals for the burst pattern.
    #[arg(long, default_value_t = 64)]
    pub(super) burst_size: u64,
    /// Maximum waiting requests per operation class, excluding executing requests.
    #[arg(long, default_value_t = 1024)]
    pub(super) queue_capacity: usize,
    /// Deadline from the original scheduled arrival; zero disables deadlines.
    #[arg(long, default_value_t = 0)]
    pub(super) request_timeout_ms: u64,
    /// Record phase timestamps for one in N requests; zero disables phase sampling.
    #[arg(long, default_value_t = 0)]
    pub(super) phase_sample_every: u64,
    /// Complete requests without Raft calls to calibrate arrival and dispatch timing.
    #[arg(long)]
    pub(super) arrival_noop: bool,
}

impl ArrivalConfig {
    pub(super) fn enabled(&self) -> bool {
        self.open_loop
            || self.phase_sample_every > 0
            || self.request_timeout_ms > 0
            || self.arrival_pattern != Pattern::Uniform
            || self.arrival_noop
    }

    pub(super) fn sampled(&self, operation: u64, stream: u64) -> bool {
        if self.phase_sample_every == 0 {
            return false;
        }
        let seed = operation ^ self.arrival_seed ^ stream;
        random_bits(seed).is_multiple_of(self.phase_sample_every)
    }
}

#[derive(Clone)]
pub(super) struct Workload {
    pub(super) config: ArrivalConfig,
    pub(super) clients: u64,
    pub(super) operations: u64,
    pub(super) rate: f64,
    pub(super) measured: bool,
    pub(super) stream_seed: u64,
    pub(super) measurement: MeasurementConfig,
    pub(super) trace_budget: usize,
    pub(super) live: Arc<Live>,
    pub(super) start: Arc<Barrier>,
    pub(super) epoch: Arc<OnceLock<Instant>>,
}

#[derive(Clone, Copy)]
pub(super) struct Arrival {
    pub(super) operation: u64,
    pub(super) scheduled: Instant,
    pub(super) released: Option<Instant>,
}

pub(super) type Task = JoinHandle<anyhow::Result<Samples>>;

type Receiver = Arc<Mutex<mpsc::Receiver<Arrival>>>;

pub(super) trait Handler: Clone + Send + Sync + 'static {
    fn run(&self, arrival: Arrival) -> impl Future<Output = anyhow::Result<Option<RequestSample>>> + Send;
}

impl Workload {
    pub(super) fn spawn<H>(&self, run: H) -> Vec<Task>
    where H: Handler {
        let mut tasks = Vec::new();
        if self.config.open_loop {
            let (tx, rx) = mpsc::channel(self.config.queue_capacity);
            let rx = Arc::new(Mutex::new(rx));
            let workload = self.clone();
            tasks.push(tokio::spawn(async move { workload.generate(tx).await }));
            for _ in 0..self.clients {
                tasks.push(self.spawn_worker(rx.clone(), run.clone()));
            }
        } else {
            for id in 0..self.clients {
                tasks.push(self.spawn_session(id, run.clone()));
            }
        }
        tasks
    }

    fn spawn_worker<H>(&self, receiver: Receiver, run: H) -> Task
    where H: Handler {
        let workload = self.clone();
        tokio::spawn(async move {
            workload.begin().await?;
            let mut samples = Samples::new(workload.measurement.bounds(), workload.trace_budget);
            loop {
                let mut queue = receiver.lock().await;
                let next = queue.recv().await;
                drop(queue);
                let Some(arrival) = next else { break };
                if let Some(sample) = run.run(arrival).await? {
                    samples.record(sample);
                } else {
                    samples.record_unmeasured();
                }
                if workload.measurement.measurement_seconds.is_some() {
                    workload.live.finish();
                }
            }
            Ok(samples)
        })
    }

    fn spawn_session<H>(&self, id: u64, run: H) -> Task
    where H: Handler {
        let workload = self.clone();
        tokio::spawn(async move {
            let epoch = workload.begin().await?;
            let mut schedule = Schedule::new(&workload, id, workload.clients, epoch);
            let bounds = workload.measurement.bounds();
            let mut samples = Samples::new(bounds, workload.trace_budget);
            while let Some(arrival) = schedule.next() {
                if !bounds.generating(epoch, arrival.scheduled) {
                    break;
                }
                if workload.rate > 0.0 {
                    wait(arrival.scheduled).await;
                }
                if !bounds.generating(epoch, arrival.scheduled) {
                    break;
                }
                if bounds.end.is_some() {
                    workload.live.offer(true);
                }
                if let Some(sample) = run.run(arrival).await? {
                    samples.record(sample);
                } else {
                    samples.record_unmeasured();
                }
                if bounds.end.is_some() {
                    workload.live.finish();
                }
                if (arrival.operation / workload.clients + 1).is_multiple_of(YIELD_BATCH) {
                    tokio::task::yield_now().await;
                }
            }
            Ok(samples)
        })
    }

    async fn begin(&self) -> anyhow::Result<Instant> {
        self.start.wait().await;
        let epoch = *self.epoch.get().context("missing arrival epoch")?;
        Ok(epoch)
    }

    async fn generate(&self, sender: mpsc::Sender<Arrival>) -> anyhow::Result<Samples> {
        let epoch = self.begin().await?;
        let bounds = self.measurement.bounds();
        let mut schedule = Schedule::new(self, 0, 1, epoch);
        let mut samples = Samples::new(bounds, self.trace_budget);
        while let Some(mut arrival) = schedule.next() {
            let offset = arrival.scheduled.saturating_duration_since(epoch);
            if bounds.end.is_some_and(|end| offset >= end) {
                break;
            }
            if bounds.deadline.is_some_and(|end| epoch.elapsed() >= end) {
                anyhow::bail!("arrival generator exceeded drain deadline before accounting all scheduled arrivals");
            }
            wait(arrival.scheduled).await;
            arrival.released = Some(Instant::now());
            let outcome = if bounds.end.is_some_and(|end| epoch.elapsed() >= end) {
                Some(Outcome::Missed)
            } else {
                match sender.try_reserve() {
                    Ok(permit) => {
                        if bounds.end.is_some() {
                            self.live.offer(true);
                        }
                        permit.send(arrival);
                        None
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => Some(Outcome::Rejected),
                    Err(mpsc::error::TrySendError::Closed(_)) => anyhow::bail!("arrival workers stopped"),
                }
            };
            if let Some(outcome) = outcome {
                if bounds.end.is_some() {
                    self.live.offer(false);
                }
                let mut sample = RequestSample::new(epoch, arrival, false)?;
                sample.outcome = outcome;
                sample.finish(epoch, arrival.scheduled)?;
                samples.record(sample);
            }
            if (arrival.operation + 1).is_multiple_of(YIELD_BATCH) {
                tokio::task::yield_now().await;
            }
        }
        Ok(samples)
    }
}

/// An independent schedule for a generator, or one sequential client session.
struct Schedule {
    pattern: Pattern,
    burst: u64,
    random: u64,
    operation: u64,
    stride: u64,
    operations: u64,
    rate: f64,
    measured: bool,
    offset: f64,
    epoch: Instant,
}

impl Schedule {
    fn new(workload: &Workload, id: u64, stride: u64, epoch: Instant) -> Self {
        Self {
            pattern: workload.config.arrival_pattern,
            burst: workload.config.burst_size,
            random: (workload.config.arrival_seed ^ workload.stream_seed).wrapping_add(id),
            operation: id,
            stride,
            operations: if workload.measurement.measurement_seconds.is_some() {
                u64::MAX
            } else {
                workload.operations
            },
            rate: workload.rate,
            measured: workload.measured,
            offset: 0.0,
            epoch,
        }
    }

    fn next(&mut self) -> Option<Arrival> {
        if self.operation >= self.operations {
            return None;
        }
        let operation = self.operation;
        self.operation += self.stride;
        let scheduled = if self.rate == 0.0 {
            if self.measured { Instant::now() } else { self.epoch }
        } else {
            self.offset = self.next_offset(operation);
            self.epoch + Duration::from_secs_f64(self.offset)
        };
        Some(Arrival {
            operation,
            scheduled,
            released: None,
        })
    }

    fn next_offset(&mut self, operation: u64) -> f64 {
        match self.pattern {
            Pattern::Uniform => operation as f64 / self.rate,
            Pattern::Burst => (operation / self.burst * self.burst) as f64 / self.rate,
            Pattern::Poisson => {
                self.random = self.random.wrapping_add(0x9e3779b97f4a7c15);
                let bits = random_bits(self.random);
                let uniform = ((bits >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
                self.offset - uniform.ln() * self.stride as f64 / self.rate
            }
        }
    }
}

pub(super) fn random_bits(mut bits: u64) -> u64 {
    bits = (bits ^ (bits >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    bits = (bits ^ (bits >> 27)).wrapping_mul(0x94d049bb133111eb);
    bits ^ (bits >> 31)
}

async fn wait(scheduled: Instant) {
    if scheduled > Instant::now() {
        let deadline = tokio::time::Instant::from_std(scheduled);
        tokio::time::sleep_until(deadline).await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::OnceLock;
    use std::time::Duration;
    use std::time::Instant;

    use clap::Parser;
    use tokio::sync::Barrier;
    use tokio::sync::mpsc;

    use super::super::Args;
    use super::Pattern;
    use super::Schedule;
    use super::Workload;

    fn workload() -> Workload {
        let args = Args::parse_from(["bench"]);
        let epoch = Arc::new(OnceLock::new());
        epoch.set(Instant::now()).unwrap();
        Workload {
            config: args.arrival,
            clients: 2,
            operations: 4,
            rate: 1000.0,
            measured: true,
            stream_seed: super::READ_STREAM,
            measurement: args.measurement,
            trace_budget: 4,
            live: Arc::default(),
            start: Arc::new(Barrier::new(1)),
            epoch,
        }
    }

    #[tokio::test]
    async fn a_stalled_receiver_retains_only_queue_capacity() -> anyhow::Result<()> {
        let mut workload = workload();
        workload.config.arrival_pattern = Pattern::Burst;
        let (tx, mut rx) = mpsc::channel(2);
        let rejected = workload.generate(tx).await?;
        let mut admitted = Vec::new();
        while let Some(arrival) = rx.recv().await {
            admitted.push(arrival.operation);
        }
        assert_eq!(vec![0, 1], admitted);
        let actual = rejected.timing(1);
        let expected = serde_json::json!(2);
        assert_eq!(expected, actual["rejected"]);
        assert_eq!(2, rejected.request_count());
        Ok(())
    }

    #[test]
    fn uniform_and_burst_schedules_keep_the_requested_arrivals() {
        let mut workload = workload();
        let epoch = *workload.epoch.get().unwrap();
        let mut uniform = Schedule::new(&workload, 1, 2, epoch);
        let mut offsets = Vec::new();
        while let Some(arrival) = uniform.next() {
            offsets.push(arrival.scheduled.duration_since(epoch));
        }
        assert_eq!(vec![Duration::from_millis(1), Duration::from_millis(3)], offsets);
        workload.config.arrival_pattern = Pattern::Burst;
        workload.config.burst_size = 2;
        let mut burst = Schedule::new(&workload, 0, 1, epoch);
        let mut offsets = Vec::new();
        while let Some(arrival) = burst.next() {
            offsets.push(arrival.scheduled.duration_since(epoch));
        }
        assert_eq!(
            vec![
                Duration::ZERO,
                Duration::ZERO,
                Duration::from_millis(2),
                Duration::from_millis(2)
            ],
            offsets
        );
    }

    #[test]
    fn poisson_arrivals_are_reproducible_and_strictly_increasing() {
        let mut workload = workload();
        workload.operations = 100;
        workload.config.arrival_pattern = Pattern::Poisson;
        let epoch = *workload.epoch.get().unwrap();
        let mut first = Schedule::new(&workload, 0, 1, epoch);
        let mut second = Schedule::new(&workload, 0, 1, epoch);
        let mut previous = epoch;
        while let Some(arrival) = first.next() {
            let replay = second.next().unwrap();
            assert_eq!(arrival.scheduled, replay.scheduled);
            assert!(arrival.scheduled > previous);
            previous = arrival.scheduled;
        }
        let remaining = second.next();
        assert!(remaining.is_none());
        workload.stream_seed = super::WRITE_STREAM;
        let mut writes = Schedule::new(&workload, 0, 1, epoch);
        let mut reads = Schedule::new(
            &Workload {
                stream_seed: super::READ_STREAM,
                ..workload
            },
            0,
            1,
            epoch,
        );
        let write_arrival = writes.next().unwrap();
        let read_arrival = reads.next().unwrap();
        assert_ne!(write_arrival.scheduled, read_arrival.scheduled);
    }

    #[test]
    fn sparse_phase_sampling_covers_every_reader() {
        let mut config = workload().config;
        config.phase_sample_every = 1024;
        let mut readers = std::collections::BTreeSet::new();
        for operation in 0..1_000_000 {
            if config.sampled(operation, super::READ_STREAM) {
                readers.insert(operation % 64);
            }
        }
        let expected: std::collections::BTreeSet<_> = (0..64).collect();
        assert_eq!(expected, readers);
    }
    #[tokio::test]
    async fn late_arrivals_are_accounted_as_missed_instead_of_disappearing() -> anyhow::Result<()> {
        let mut workload = workload();
        workload.measurement.measurement_seconds = Some(0.004);
        workload.measurement.warmup_seconds = 0.0;
        workload.measurement.drain_seconds = 1.0;
        let epoch = Arc::new(OnceLock::new());
        epoch.set(Instant::now() - Duration::from_millis(10)).unwrap();
        workload.epoch = epoch;
        let (tx, mut rx) = mpsc::channel(2);
        let samples = workload.generate(tx).await?;
        let report = samples.timing(1);
        let expected = serde_json::json!(4);
        assert_eq!(expected, report["offered"]);
        let expected = serde_json::json!(0);
        assert_eq!(expected, report["admitted"]);
        let expected = serde_json::json!(4);
        assert_eq!(expected, report["missed"]);
        let admitted = rx.recv().await;
        assert!(admitted.is_none());
        Ok(())
    }
}
