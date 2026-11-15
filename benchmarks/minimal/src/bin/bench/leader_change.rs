use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use bench_minimal::network::BenchRaft;
use bench_minimal::network::Router;
use bench_minimal::store::ClientRequest;
use bench_minimal::store::StateMachineStore;
use openraft::ServerState;
use serde_json::Value;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// The node that leads before the transfer.
pub(super) const OLD_LEADER: u64 = 0;
/// The node that leads after the transfer.
pub(super) const NEW_LEADER: u64 = 1;

/// How long a rejected client waits for the leader to change before it retries anyway.
const RETRY_DELAY: Duration = Duration::from_millis(1);
/// How long one transfer attempt may take before it is triggered again.
const ATTEMPT_TIMEOUT: Duration = Duration::from_millis(5);
/// How long the whole transfer may take.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(10);

/// Lets clients follow leadership when it moves from node 0 to node 1 mid-run.
///
/// Clients send to the node they last saw as the leader. When a node rejects a request because it
/// is not the leader or is handing leadership over, the client waits until the new leader has
/// taken over, or for [`RETRY_DELAY`], and then retries on the current leader.
#[derive(Clone)]
pub(super) struct Leaders {
    router: Router,
    machines: Arc<BTreeMap<u64, Arc<StateMachineStore>>>,
    current: watch::Receiver<u64>,
    publish: Arc<watch::Sender<u64>>,
}

impl Leaders {
    /// `machines` holds the state machine of every node, for queries on the current leader.
    pub(super) fn new(router: Router, machines: BTreeMap<u64, Arc<StateMachineStore>>) -> Self {
        let (publish, current) = watch::channel(OLD_LEADER);
        Self {
            router,
            machines: Arc::new(machines),
            current,
            publish: Arc::new(publish),
        }
    }

    /// The node that clients send requests to, and its handle.
    pub(super) fn current(&self) -> (u64, BenchRaft) {
        let id = *self.current.borrow();
        let raft = self.router.get_raft(id);
        (id, raft)
    }

    pub(super) fn machine(&self, node: u64) -> anyhow::Result<&StateMachineStore> {
        let machine = self.machines.get(&node).with_context(|| format!("no state machine for node {node}"))?;
        Ok(machine)
    }

    /// Waits after `tried` rejected a request, until the leader changes or [`RETRY_DELAY`] passes.
    pub(super) async fn retry_later(&self, tried: u64) {
        let mut current = self.current.clone();
        let changed = current.wait_for(|leader| *leader != tried);
        let _ = tokio::time::timeout(RETRY_DELAY, changed).await;
    }

    /// Writes once, retrying on the current leader until a leader accepts the write.
    pub(super) async fn client_write(&self) -> anyhow::Result<()> {
        loop {
            let (node, raft) = self.current();
            let Err(error) = raft.client_write(ClientRequest {}).await else {
                return Ok(());
            };
            if error.forward_to_leader().is_none() {
                return Err(error.into());
            }
            self.retry_later(node).await;
        }
    }

    /// Transfers leadership to [`NEW_LEADER`] `after` the epoch, and points clients at it once it
    /// has taken over.
    pub(super) fn transfer_after(&self, epoch: Instant, after: Duration) -> JoinHandle<anyhow::Result<LeaderChange>> {
        let leaders = self.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::from_std(epoch + after);
            tokio::time::sleep_until(deadline).await;
            let transferred_ns = nanos_since(epoch)?;
            let transfer = leaders.transfer();
            tokio::time::timeout(TRANSFER_TIMEOUT, transfer).await.context("leadership transfer timed out")??;
            leaders.publish.send_replace(NEW_LEADER);
            let elected_ns = nanos_since(epoch)?;
            Ok(LeaderChange {
                transferred_ns,
                elected_ns,
            })
        })
    }

    /// Triggers the transfer until the new leader takes over.
    ///
    /// The target ignores a transfer while its log is behind the old leader's; the old leader then
    /// stops proposing, so a later attempt finds the target caught up.
    async fn transfer(&self) -> anyhow::Result<()> {
        let old = self.router.get_raft(OLD_LEADER);
        let new = self.router.get_raft(NEW_LEADER);
        loop {
            old.trigger().transfer_leader(NEW_LEADER).await?;
            let wait = new.wait(Some(ATTEMPT_TIMEOUT));
            let leading = wait.state(ServerState::Leader, "new leader takes over").await;
            if leading.is_ok() {
                return Ok(());
            }
        }
    }
}

/// When leadership moved, in nanoseconds since the arrival epoch.
pub(super) struct LeaderChange {
    transferred_ns: u64,
    elected_ns: u64,
}

impl LeaderChange {
    /// Reports the election and the reads around it.
    ///
    /// `reads` holds each successful read as its start and its latency in nanoseconds since the
    /// epoch, with the node that served it.
    pub(super) fn report(&self, reads: &[(u64, u64, u64)]) -> Value {
        let elected = self.elected_ns;
        let first_read = reads
            .iter()
            .filter(|(_, _, node)| *node == NEW_LEADER)
            .map(|(started, latency, _)| started + latency)
            .filter(|finished| *finished >= elected)
            .min()
            .map(|finished| finished - elected);
        let window = elected..elected.saturating_add(1_000_000_000);
        let mut first_second: Vec<u64> = reads
            .iter()
            .filter(|(started, _, _)| window.contains(started))
            .map(|(_, latency, _)| *latency)
            .collect();
        first_second.sort_unstable();
        let summary = summary(&first_second);
        serde_json::json!({
            "transferred_ns": self.transferred_ns, "elected_ns": elected,
            "election_ns": elected - self.transferred_ns,
            "first_read_after_election_ns": first_read, "first_second_reads": summary,
        })
    }
}

fn summary(sorted: &[u64]) -> Value {
    if sorted.is_empty() {
        return Value::Null;
    }
    let rank = |percentile: usize| sorted[(sorted.len() * percentile).div_ceil(100) - 1];
    serde_json::json!({"samples": sorted.len(), "p50_ns": rank(50), "p99_ns": rank(99)})
}

fn nanos_since(epoch: Instant) -> anyhow::Result<u64> {
    let nanos = epoch.elapsed().as_nanos();
    Ok(u64::try_from(nanos)?)
}

#[cfg(test)]
mod tests {
    use super::LeaderChange;
    use super::NEW_LEADER;
    use super::OLD_LEADER;

    #[test]
    fn reads_are_measured_from_the_election() {
        let change = LeaderChange {
            transferred_ns: 1_000,
            elected_ns: 4_000,
        };
        let reads = [
            // Finished on the old leader after the election: not the first read on the new one.
            (3_000, 2_000, OLD_LEADER),
            // Started before the election, finished on the new leader after it.
            (3_500, 1_500, NEW_LEADER),
            (4_500, 100, NEW_LEADER),
            // Started after the first second.
            (1_000_004_000, 10, NEW_LEADER),
        ];
        let actual = change.report(&reads);
        let expected = serde_json::json!({
            "transferred_ns": 1_000, "elected_ns": 4_000, "election_ns": 3_000,
            "first_read_after_election_ns": 600,
            "first_second_reads": {"samples": 1, "p50_ns": 100, "p99_ns": 100},
        });
        assert_eq!(expected, actual);
    }
}
