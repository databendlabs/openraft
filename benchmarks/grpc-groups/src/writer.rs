use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::FuturesUnordered;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use raft_kv_memstore_grpc::protobuf as raftpb;
use tokio::time::Instant;
use tokio::time::MissedTickBehavior;

use crate::Raft;
use crate::write_stats::WriteStats;

/// How often the writer issues the writes that have come due. Each write keeps its own scheduled
/// time, which its latency counts from, so the tick sets only how many writes go out together.
const TICK: Duration = Duration::from_millis(1);

/// The writer skips a write that came due longer ago than this when it gets to it. Only a writer
/// that could not run falls this far behind, as in a stopped node, whose writer would otherwise
/// send the writes of the whole stop at the resume, to groups it no longer leads.
const MAX_LATE: Duration = Duration::from_secs(1);

/// Write `rate` entries a second to each of `groups` whose member on this node leads it at the
/// scheduled time, until the node exits, and record each outcome in `stats`.
///
/// The schedule is open loop: the writes of all groups take turns at even intervals, and a write
/// goes out when its time comes, whether or not the earlier ones have committed. A stalled leader
/// therefore shows up as latency rather than as fewer writes. A writer that resumes after a stop
/// skips the writes that came due more than [`MAX_LATE`] before.
pub(crate) async fn write_open_loop(groups: Vec<Raft>, rate: u64, stats: Arc<WriteStats>) {
    if groups.is_empty() || rate == 0 {
        return;
    }
    let receivers = groups.iter().map(|raft| raft.server_metrics()).collect::<Vec<_>>();
    // At least 1 ns, so that the schedule moves on even at an absurd rate.
    let slot_nanos = (1_000_000_000 / rate.saturating_mul(groups.len() as u64)).max(1);

    let start = Instant::now();
    let due = |slot: u64| start + Duration::from_nanos(slot_nanos * slot);
    let mut next_slot = 0u64;

    let mut inflight = FuturesUnordered::new();
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let now = Instant::now();
                if let Some(earliest) = now.checked_sub(MAX_LATE) {
                    let first = earliest.saturating_duration_since(start).as_nanos() / u128::from(slot_nanos);
                    next_slot = next_slot.max(u64::try_from(first).unwrap_or(u64::MAX));
                }
                while due(next_slot) <= now {
                    let index = (next_slot % groups.len() as u64) as usize;
                    if receivers[index].borrow_watched().state == ServerState::Leader {
                        inflight.push(write(groups[index].clone(), due(next_slot), next_slot, stats.clone()));
                    }
                    next_slot += 1;
                }
            }
            Some(()) = inflight.next(), if !inflight.is_empty() => {}
        }
    }
}

/// Write one entry to `raft`, and record when it committed, from `due`, or that it failed.
async fn write(raft: Raft, due: Instant, slot: u64, stats: Arc<WriteStats>) {
    // Every write sets the same key, so that the state machine of a group stays small.
    let request = raftpb::SetRequest {
        key: "key".to_string(),
        value: slot.to_string(),
    };
    match raft.client_write(request).await {
        Ok(_) => stats.committed(due.elapsed()),
        Err(_) => stats.rejected(),
    }
}
