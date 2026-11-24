use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use openraft::async_runtime::WatchReceiver;
use openraft::type_config::TypeConfigExt;
use tokio::task::JoinHandle;

use crate::C;
use crate::Raft;
use crate::duration_histogram::DurationHistogram;
use crate::process_usage;
use crate::protobuf as pb;
use crate::router_stats::RouterCounts;
use crate::router_stats::RouterStats;

/// How often a window samples the acknowledgement ages of the leaders.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

/// One measurement window of a node: what its router had counted at the start, and the
/// acknowledgement ages of its leaders, sampled since.
pub(crate) struct Window {
    started: Instant,
    cpu_secs: f64,
    counts: RouterCounts,
    samples: Arc<Mutex<AckSamples>>,
    sampler: JoinHandle<()>,
}

/// The acknowledgement ages that a window sampled.
#[derive(Default)]
struct AckSamples {
    /// `now - heartbeat[follower]` of every follower that is up.
    ack_age_us: DurationHistogram,

    /// The largest `now - last_quorum_acked`.
    quorum_age_max_us: u64,
}

impl Window {
    /// Start a window on node `id`: discard the heartbeat round trips that `stats` recorded so
    /// far, and sample the leaders among `groups`, leaving the followers on the nodes in `down`
    /// out.
    pub(crate) fn begin(id: u64, groups: &[Raft], stats: &RouterStats, down: BTreeSet<u64>) -> anyhow::Result<Self> {
        let cpu_secs = process_usage::cpu_secs().context("read the CPU time of the node")?;
        stats.take_hb_rtt();
        let counts = stats.counts();

        let samples = Arc::new(Mutex::new(AckSamples::default()));
        let sampler = tokio::spawn(sample_ack_ages(id, groups.to_vec(), down, samples.clone()));

        Ok(Self {
            started: Instant::now(),
            cpu_secs,
            counts,
            samples,
            sampler,
        })
    }

    /// End the window, and report it with `terms`, the terms of the node's members at the end.
    pub(crate) fn end(self, stats: &RouterStats, terms: pb::Terms) -> anyhow::Result<pb::WindowReport> {
        self.sampler.abort();

        let secs = self.started.elapsed().as_secs_f64();
        let cpu_secs = process_usage::cpu_secs().context("read the CPU time of the node")? - self.cpu_secs;
        let memory_kib = process_usage::memory_kib().context("read the memory of the node")?;
        let tasks = tokio::runtime::Handle::current().metrics().num_alive_tasks() as u64;
        let counts = stats.counts().since(&self.counts);
        let hb_rtt_us = stats.take_hb_rtt();
        let samples = self.samples.lock().expect("no thread panics while it holds the lock");

        Ok(pb::WindowReport {
            secs,
            cpu_secs,
            memory_kib,
            tasks,
            appends: counts.appends,
            rpcs: counts.rpcs,
            votes: counts.votes,
            hb_failed: counts.hb_failed,
            hb_rtt_us: Some(hb_rtt_us.to_pb()),
            ack_age_us: Some(samples.ack_age_us.to_pb()),
            quorum_age_max_us: samples.quorum_age_max_us,
            terms: Some(terms),
        })
    }
}

/// Sample the acknowledgement ages of the members of `groups` that lead their group, every
/// [`SAMPLE_INTERVAL`], until the window aborts this task.
///
/// A leader's `heartbeat` metrics map each member to the send time of the last request that the
/// member acknowledged, and include the leader itself, which `id` leaves out.
async fn sample_ack_ages(id: u64, groups: Vec<Raft>, down: BTreeSet<u64>, samples: Arc<Mutex<AckSamples>>) {
    let receivers = groups.iter().map(|raft| raft.data_metrics()).collect::<Vec<_>>();
    let mut ticker = tokio::time::interval(SAMPLE_INTERVAL);

    loop {
        ticker.tick().await;
        let now = C::now();
        let mut samples = samples.lock().expect("no thread panics while it holds the lock");

        for receiver in &receivers {
            let metrics = receiver.borrow_watched();
            // Only a leader has heartbeat metrics.
            let Some(heartbeat) = &metrics.heartbeat else {
                continue;
            };

            let followers = heartbeat.iter().filter(|(node_id, _)| **node_id != id && !down.contains(node_id));
            for (_, sent) in followers {
                if let Some(sent) = sent {
                    samples.ack_age_us.record(micros_since(**sent, now));
                }
            }

            if let Some(acked) = &metrics.last_quorum_acked {
                let age = micros_since(**acked, now);
                samples.quorum_age_max_us = samples.quorum_age_max_us.max(age);
            }
        }
    }
}

/// Return the microseconds from `earlier` to `now`, or 0 if `earlier` is later.
fn micros_since<I>(earlier: I, now: I) -> u64
where I: openraft::Instant {
    if earlier > now {
        return 0;
    }
    u64::try_from((now - earlier).as_micros()).unwrap_or(u64::MAX)
}
