//! A burst of client writes to every group, before the settle period and the measurement window.

use std::num::NonZeroUsize;
use std::time::Duration;
use std::time::Instant;

use base2histogram::Histogram;
use futures::StreamExt;
use futures::future::join_all;
use futures::stream;
use openraft::RaftMetrics;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardReason;
use openraft::errors::RaftError;
use openraft_memstore::ClientRequest;

use crate::C;
use crate::Raft;

/// The bucket width of the latency histogram: 32 buckets per power of two, so that a percentile is
/// within about 1.6% of the latency it estimates.
const LATENCY_WIDTH: usize = 6;

/// How long to wait before retrying a write that a leader rejected because no quorum acknowledged
/// it recently.
const LEASE_RETRY_DELAY: Duration = Duration::from_millis(10);

/// What a burst of writes measured.
#[derive(Debug, Clone)]
pub(crate) struct WriteBurst {
    /// The writes made, summed over the groups.
    pub(crate) writes: u64,

    /// The writes retried because the leader's lease had expired, summed over the groups.
    pub(crate) retries: u64,

    /// Seconds from the first write to the last response.
    pub(crate) secs: f64,

    /// The latency of each write, retries included, in microseconds.
    pub(crate) latency: Histogram,

    /// The largest latency, in microseconds.
    pub(crate) max_micros: u64,
}

impl WriteBurst {
    /// Write `writes` entries to the leader of every group, `depth` at a time per group, and
    /// measure the latency of each write.
    ///
    /// The first member of a group is its leader. It starts once a quorum has acknowledged every
    /// leader, since a leader rejects writes until then. A write that a leader rejects for an
    /// expired lease is retried; any other failed write panics, as when the leader of a group
    /// changes during the burst.
    pub(crate) async fn run(groups: &[Vec<Raft>], writes: u64, depth: NonZeroUsize) -> Self {
        let leases = groups.iter().map(|group| async move {
            let wait = group[0].wait(Some(Duration::from_secs(60)));
            let acked = |m: &RaftMetrics<C>| m.last_quorum_acked.is_some();
            wait.metrics(acked, "a quorum acknowledges the leader")
                .await
                .expect("a quorum acknowledges the leader");
        });
        join_all(leases).await;

        let bursts = groups.iter().zip(0u64..).map(|(group, group_id)| {
            let leader = &group[0];
            stream::iter(0..writes)
                .map(move |serial| write(leader, group_id, serial))
                .buffer_unordered(depth.get())
        });

        let started = Instant::now();
        let empty = Self {
            writes: 0,
            retries: 0,
            secs: 0.0,
            latency: Histogram::with_log_scale(LATENCY_WIDTH, 1),
            max_micros: 0,
        };
        let mut burst = stream::select_all(bursts)
            .fold(empty, |mut burst, (micros, retries)| async move {
                burst.writes += 1;
                burst.retries += retries;
                burst.latency.record(micros);
                burst.max_micros = burst.max_micros.max(micros);
                burst
            })
            .await;
        burst.secs = started.elapsed().as_secs_f64();
        burst
    }

    /// The writes per second over the burst.
    pub(crate) fn writes_per_sec(&self) -> f64 {
        self.writes as f64 / self.secs
    }

    /// The latency at percentile `p`, in `[0.0, 1.0]`, in microseconds.
    pub(crate) fn percentile_micros(&self, p: f64) -> u64 {
        self.latency.percentile(p)
    }
}

/// Write entry `serial` of group `group_id` to `leader`, and return its latency in microseconds and
/// the number of times it was retried.
///
/// A leader that no quorum has acknowledged recently rejects the write with
/// `ForwardReason::LeaseExpired` before appending it, so retrying it on the same leader, as the
/// docs tell a client to, writes it once.
async fn write(leader: &Raft, group_id: u64, serial: u64) -> (u64, u64) {
    let request = ClientRequest {
        client: format!("group-{group_id}"),
        serial,
        status: format!("value-{serial}"),
    };

    let started = Instant::now();
    let mut retries = 0;
    loop {
        match leader.client_write(request.clone()).await {
            Ok(_) => return (started.elapsed().as_micros() as u64, retries),
            Err(RaftError::APIError(ClientWriteError::ForwardToLeader(forward)))
                if forward.reason == ForwardReason::LeaseExpired =>
            {
                retries += 1;
                tokio::time::sleep(LEASE_RETRY_DELAY).await;
            }
            Err(e) => panic!("write {serial} to the leader of group {group_id}: {e}"),
        }
    }
}
