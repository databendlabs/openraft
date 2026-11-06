//! The in-process cluster that every scenario runs on.

mod kv_type_config;
mod written;

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use log_mem::LogStore;
use network_v2_mem::Registry;
use openraft::Config;
use openraft::LogId;
use openraft::LogIdOptionExt;
use openraft::Raft;
use openraft::RaftMetrics;
use openraft::ServerState;
use openraft::SnapshotPolicy;
use openraft::StoredMembership;
use openraft::alias::LogIdOf;
use openraft::async_runtime::WatchReceiver;
use openraft::raft::ClientWriteResponse;
use sm_mem::StateMachineStore;
use types_kv::Request;
use types_kv::Response;
use types_kv::VersionedValue;

pub use self::kv_type_config::KvTypeConfig;
pub use self::written::Written;
use crate::Policy;

/// Node 1 initializes the cluster and leads it in every phase of every scenario.
pub const LEADER: u64 = 1;

/// Bounds a whole scenario.
///
/// It is shorter than the leader lease, so the leader keeps its lease for the whole scenario.
pub const SCENARIO_TIMEOUT: Duration = Duration::from_secs(30);

/// Bounds one Raft API call, such as a write or a membership change.
pub const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

/// Bounds one wait on the metrics of one node, such as for it to apply a log entry.
pub const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a scenario watches a write that a fault holds back, to see that it stays pending.
///
/// A timeout only shows that no response arrived within this interval: the write is still
/// pending, and commits once the fault heals.
pub const BLOCKED_WRITE_INTERVAL: Duration = Duration::from_millis(500);

/// The leader lease, in milliseconds. OpenRaft sets it to `Config::election_timeout_max`.
///
/// A leader accepts a write only while a write quorum acknowledged it within the lease. With
/// heartbeats off, only replication renews those acknowledgments, so a lease that outlasts every
/// scenario keeps the leader writable between phases.
const LEADER_LEASE_MS: u64 = 60_000;

/// A node of a [`Cluster`]: its Raft handle and its state machine.
struct Node<C>
where C: KvTypeConfig
{
    raft: Raft<C, StateMachineStore<C>>,
    state_machine: StateMachineStore<C>,
}

/// The nodes of one scenario, connected by the in-process transport of `network-v2-mem`.
///
/// Every node starts with an empty `log-mem` log store and an empty `sm-mem` state machine, and
/// keeps both while it is isolated. A failed call, wait or check returns an error that ends with
/// [`report()`](Self::report).
pub struct Cluster<C>
where C: KvTypeConfig
{
    policy: Policy,
    registry: Arc<Registry<C, StateMachineStore<C>>>,
    nodes: BTreeMap<u64, Node<C>>,
}

impl<C> Cluster<C>
where C: KvTypeConfig
{
    /// Starts a node with empty stores for each of `ids`, and registers it with the transport.
    ///
    /// Ticks are off: no node sends a heartbeat or starts an election on its own, so node 1 stays
    /// the leader once `initialize()` elects it. No node builds a snapshot.
    pub async fn new(policy: Policy, ids: impl IntoIterator<Item = u64>) -> anyhow::Result<Self> {
        let config = Config {
            cluster_name: format!("membership-{}", policy),
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            snapshot_policy: SnapshotPolicy::Never,
            election_timeout_min: LEADER_LEASE_MS / 2,
            election_timeout_max: LEADER_LEASE_MS,
            ..Default::default()
        };
        let config = Arc::new(config.validate()?);

        let registry = Arc::new(Registry::default());
        let mut nodes = BTreeMap::new();

        for id in ids {
            let state_machine = StateMachineStore::default();
            let network = registry.network_factory(id);
            let raft = Raft::new(id, config.clone(), network, LogStore::default(), state_machine.clone()).await?;
            registry.register(id, raft.clone());
            nodes.insert(id, Node { raft, state_machine });
        }

        Ok(Self {
            policy,
            registry,
            nodes,
        })
    }

    /// Returns the Raft handle of node `id`.
    pub fn raft(&self, id: u64) -> &Raft<C, StateMachineStore<C>> {
        &self.node(id).raft
    }

    /// Returns the membership that node `id` uses, the one of its last membership log entry.
    pub fn membership(&self, id: u64) -> C::Membership {
        let metrics = self.raft(id).metrics();
        let metrics = metrics.borrow_watched();
        metrics.membership_config.membership().clone()
    }

    /// Awaits `call`, one Raft API call, for at most [`OPERATION_TIMEOUT`].
    pub async fn call<T, E>(&self, what: &str, call: impl Future<Output = Result<T, E>>) -> anyhow::Result<T>
    where E: std::error::Error + Send + Sync + 'static {
        let Ok(res) = tokio::time::timeout(OPERATION_TIMEOUT, call).await else {
            anyhow::bail!(
                "{}: no response within {:?}\n{}",
                what,
                OPERATION_TIMEOUT,
                self.report()
            );
        };
        res.with_context(|| format!("{}\n{}", what, self.report()))
    }

    /// Waits until node `id` leads and has applied the blank entry of its term.
    ///
    /// `initialize()` writes the membership at log index 0, and the leader it elects writes a
    /// blank entry at index 1. Until the leader commits an entry of its own term, it rejects a
    /// membership change. Returns the log ID of the blank entry.
    pub async fn wait_initialized(&self, id: u64) -> anyhow::Result<LogIdOf<C>> {
        let what = "leads and applies its blank entry at log index 1";
        let metrics = self
            .wait(id, what, |m| {
                m.state == ServerState::Leader && m.last_applied.index() >= Some(1)
            })
            .await?;
        metrics.last_applied.with_context(|| format!("node {} {}\n{}", id, what, self.report()))
    }

    /// Waits until each node of `ids` has applied `log_id`.
    pub async fn wait_applied(&self, ids: &[u64], log_id: &LogIdOf<C>) -> anyhow::Result<()> {
        for id in ids {
            let what = format!("applies {}", log_id);
            self.wait(*id, &what, |m| m.last_applied.as_ref() >= Some(log_id)).await?;
        }
        Ok(())
    }

    /// Waits until the log of each node of `ids` holds the entry at `index`.
    pub async fn wait_log(&self, ids: &[u64], index: u64) -> anyhow::Result<()> {
        for id in ids {
            let what = format!("appends log index {}", index);
            self.wait(*id, &what, |m| m.last_log_index >= Some(index)).await?;
        }
        Ok(())
    }

    /// Writes `key = value` through the leader, waits until each node of `members` applies it,
    /// and checks their values.
    ///
    /// The write must be the entry at log index `index`; [`check_write()`](Self::check_write)
    /// checks its response.
    pub async fn write(
        &self,
        key: &'static str,
        value: &'static str,
        index: u64,
        members: &[u64],
    ) -> anyhow::Result<Written<C>> {
        let what = format!("write {}={}", key, value);
        let leader = self.raft(LEADER);
        let resp = self.call(&what, leader.client_write(Request::set(key, value))).await?;

        let written = self.check_write(resp, key, value, index)?;
        self.wait_applied(members, &written.log_id).await?;
        self.check_values(members, &[&written]).await?;
        Ok(written)
    }

    /// Checks the complete response of the write of `key = value`.
    ///
    /// The entry must be at log index `index` in the term of the leader, and the state machine
    /// must have returned `value` with version `index`. A normal entry carries no membership.
    pub fn check_write(
        &self,
        resp: ClientWriteResponse<C>,
        key: &'static str,
        value: &'static str,
        index: u64,
    ) -> anyhow::Result<Written<C>> {
        let want_log_id = self.leader_log_id(index)?;
        let want_data = Response::new(value, index);

        anyhow::ensure!(
            resp.log_id == want_log_id && resp.data == want_data && resp.membership.is_none(),
            "write {}={}: got log id {}, data {:?}, membership {}; want log id {}, data {:?}, no membership\n{}",
            key,
            value,
            resp.log_id,
            resp.data,
            display_option(&resp.membership),
            want_log_id,
            want_data,
            self.report()
        );

        Ok(Written {
            key,
            value,
            log_id: resp.log_id,
        })
    }

    /// Checks the complete response of a membership change, then waits until each node of
    /// `members` has applied it and uses `membership`.
    ///
    /// The response must carry `membership`, the uniform config, at log index `index` in the term
    /// of the leader, with the empty response that the state machine returns for a membership
    /// entry. Returns the log ID of that entry.
    pub async fn check_membership(
        &self,
        resp: ClientWriteResponse<C>,
        index: u64,
        membership: &C::Membership,
        members: &[u64],
    ) -> anyhow::Result<LogIdOf<C>> {
        let want_log_id = self.leader_log_id(index)?;
        let want_data = Response::none();

        anyhow::ensure!(
            resp.log_id == want_log_id && resp.data == want_data && resp.membership.as_ref() == Some(membership),
            "membership change: got log id {}, data {:?}, membership {}; want log id {}, data {:?}, membership {}\n{}",
            resp.log_id,
            resp.data,
            display_option(&resp.membership),
            want_log_id,
            want_data,
            membership,
            self.report()
        );

        let want = StoredMembership::new(Some(resp.log_id.clone()), membership.clone());
        for id in members {
            let what = format!("applies membership {}", want);
            self.wait(*id, &what, |m| {
                m.last_applied.as_ref() >= Some(&resp.log_id) && *m.membership_config == want
            })
            .await?;
        }

        Ok(resp.log_id)
    }

    /// Checks that each node of `ids` holds every write of `written`, at the version of its log
    /// index.
    pub async fn check_values(&self, ids: &[u64], written: &[&Written<C>]) -> anyhow::Result<()> {
        for id in ids {
            for w in written {
                let got = self.node(*id).state_machine.get(w.key).await;
                let want = VersionedValue {
                    value: w.value.to_string(),
                    version: w.log_id.index(),
                };
                anyhow::ensure!(
                    got.as_ref() == Some(&want),
                    "node {}: {} = {:?}, want {:?}\n{}",
                    id,
                    w.key,
                    got,
                    want,
                    self.report()
                );
            }
        }
        Ok(())
    }

    /// Checks that no node of `ids` holds the key of `written`: none of them received the write.
    pub async fn check_missing(&self, ids: &[u64], written: &Written<C>) -> anyhow::Result<()> {
        for id in ids {
            let got = self.node(*id).state_machine.get(written.key).await;
            anyhow::ensure!(
                got.is_none(),
                "isolated node {} holds {} = {:?}\n{}",
                id,
                written.key,
                got,
                self.report()
            );
        }
        Ok(())
    }

    /// Checks that no node of `ids` has committed the entry at `index`.
    pub fn check_uncommitted(&self, ids: &[u64], index: u64) -> anyhow::Result<()> {
        for id in ids {
            let metrics = self.raft(*id).metrics();
            let committed = metrics.borrow_watched().local_committed.index();
            anyhow::ensure!(
                committed < Some(index),
                "node {} committed log index {:?}, want below {}\n{}",
                id,
                committed,
                index,
                self.report()
            );
        }
        Ok(())
    }

    /// Cuts each node of `ids` off from every other node. The nodes keep running with their
    /// stores.
    pub fn isolate(&self, ids: &[u64]) {
        self.registry.isolate(ids.iter().copied());
    }

    /// Reconnects every isolated node, and wakes the leader's replication to them.
    ///
    /// `reset_backoff()` ends the backoff that the failed RPCs started, and `heartbeat()` sends
    /// the leader's committed log ID at once.
    pub async fn heal(&self) -> anyhow::Result<()> {
        let healed = self.registry.heal();
        let trigger = self.raft(LEADER).trigger();
        trigger.reset_backoff(healed).await?;
        trigger.heartbeat().await?;
        Ok(())
    }

    /// Shuts down every node.
    pub async fn shutdown(self) -> anyhow::Result<()> {
        for (id, node) in &self.nodes {
            let res = node.raft.shutdown().await;
            res.map_err(|e| anyhow::anyhow!("shut down node {}: {}", id, e))?;
        }
        Ok(())
    }

    /// Describes every node: its server state, term, committed and applied log IDs, and its
    /// membership.
    pub fn report(&self) -> String {
        let isolated: Vec<u64> = self.nodes.keys().copied().filter(|id| self.registry.is_isolated(id)).collect();
        let mut report = format!("{} cluster, isolated {:?}:", self.policy, isolated);

        for (id, node) in &self.nodes {
            let metrics = node.raft.metrics();
            let m = metrics.borrow_watched();
            report.push_str(&format!(
                "\n  node {}: {:?}, term {}, committed {}, applied {}, membership {}",
                id,
                m.state,
                m.current_term,
                display_option(&m.local_committed),
                display_option(&m.last_applied),
                m.membership_config
            ));
        }
        report
    }

    /// Returns the log ID of the entry at `index` in the term of the leader, node 1.
    fn leader_log_id(&self, index: u64) -> anyhow::Result<LogIdOf<C>> {
        let leader = self.raft(LEADER).as_leader();
        let leader = leader.with_context(|| format!("node {} must lead\n{}", LEADER, self.report()))?;
        Ok(LogId::new(leader.to_committed_leader_id(), index))
    }

    /// Waits until the metrics of node `id` satisfy `condition`.
    async fn wait<F>(&self, id: u64, what: &str, condition: F) -> anyhow::Result<RaftMetrics<C>>
    where F: Fn(&RaftMetrics<C>) -> bool + Send {
        let wait = self.raft(id).wait(Some(OBSERVATION_TIMEOUT));
        let res = wait.metrics(condition, format!("node {} {}", id, what)).await;
        res.with_context(|| self.report())
    }

    fn node(&self, id: u64) -> &Node<C> {
        let Some(node) = self.nodes.get(&id) else {
            panic!("node {} is not in the {} cluster", id, self.policy);
        };
        node
    }
}

/// Displays `value`, or `none`.
fn display_option<T>(value: &Option<T>) -> String
where T: fmt::Display {
    match value {
        Some(value) => value.to_string(),
        None => "none".to_string(),
    }
}
