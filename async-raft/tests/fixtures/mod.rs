//! Fixtures for testing Raft.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_raft::async_trait::async_trait;
use async_raft::error::{ChangeConfigError, ClientReadError, ClientWriteError};
use async_raft::raft::ClientWriteRequest;
use async_raft::raft::MembershipConfig;
use async_raft::raft::{AppendEntriesRequest, AppendEntriesResponse};
use async_raft::raft::{InstallSnapshotRequest, InstallSnapshotResponse};
use async_raft::raft::{VoteRequest, VoteResponse};
use async_raft::storage::RaftStorage;
use async_raft::{Config, NodeId, Raft, RaftMetrics, RaftNetwork, State};
use memstore::{ClientRequest as MemClientRequest, ClientResponse as MemClientResponse, MemStore};
use tokio::sync::RwLock;
use tracing_subscriber::prelude::*;

/// A concrete Raft type used during testing.
pub type MemRaft = Raft<MemClientRequest, MemClientResponse, RaftRouter, MemStore>;

/// Initialize the tracing system.
pub fn init_tracing() {
    let fmt_layer = tracing_subscriber::fmt::Layer::default()
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::FULL)
        .with_ansi(false);
    let subscriber = tracing_subscriber::Registry::default()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(fmt_layer);
    tracing::subscriber::set_global_default(subscriber).expect("error setting global tracing subscriber");
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

/// A type which emulates a network transport and implements the `RaftNetwork` trait.
pub struct RaftRouter {
    /// The Raft runtime config which all nodes are using.
    config: Arc<Config>,
    /// The table of all nodes currently known to this router instance.
    routing_table: RwLock<BTreeMap<NodeId, (MemRaft, Arc<MemStore>)>>,
    /// Nodes which are isolated can neither send nor receive frames.
    isolated_nodes: RwLock<HashSet<NodeId>>,
}

impl RaftRouter {
    /// Create a new instance.
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            routing_table: Default::default(),
            isolated_nodes: Default::default(),
        }
    }

    /// Create and register a new Raft node bearing the given ID.
    pub async fn new_raft_node(self: &Arc<Self>, id: NodeId) {
        let memstore = Arc::new(MemStore::new(id));
        let node = Raft::new(id, self.config.clone(), self.clone(), memstore.clone());
        let mut rt = self.routing_table.write().await;
        rt.insert(id, (node, memstore));
    }

    /// Remove the target node from the routing table & isolation.
    pub async fn remove_node(&self, id: NodeId) -> Option<(MemRaft, Arc<MemStore>)> {
        let mut rt = self.routing_table.write().await;
        let opt_handles = rt.remove(&id);
        let mut isolated = self.isolated_nodes.write().await;
        isolated.remove(&id);

        opt_handles
    }

    /// Initialize all nodes based on the config in the routing table.
    pub async fn initialize_from_single_node(&self, node: NodeId) -> Result<()> {
        tracing::info!({ node }, "initializing cluster from single node");
        let rt = self.routing_table.read().await;
        let members: HashSet<NodeId> = rt.keys().cloned().collect();
        rt.get(&node)
            .ok_or_else(|| anyhow!("node {} not found in routing table", node))?
            .0
            .initialize(members.clone())
            .await?;
        Ok(())
    }

    /// Initialize cluster with specified node ids.
    pub async fn initialize_with(&self, node: NodeId, members: HashSet<NodeId>) -> Result<()> {
        tracing::info!({ node }, "initializing cluster from single node");
        let rt = self.routing_table.read().await;
        rt.get(&node)
            .ok_or_else(|| anyhow!("node {} not found in routing table", node))?
            .0
            .initialize(members.clone())
            .await?;
        Ok(())
    }

    /// Isolate the network of the specified node.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn isolate_node(&self, id: NodeId) {
        self.isolated_nodes.write().await.insert(id);
    }

    /// Get a payload of the latest metrics from each node in the cluster.
    pub async fn latest_metrics(&self) -> Vec<RaftMetrics> {
        let rt = self.routing_table.read().await;
        let mut metrics = vec![];
        for node in rt.values() {
            metrics.push(node.0.metrics().borrow().clone());
        }
        metrics
    }

    /// Get the ID of the current leader.
    pub async fn leader(&self) -> Option<NodeId> {
        let isolated = self.isolated_nodes.read().await;
        self.latest_metrics().await.into_iter().find_map(|node| {
            if node.current_leader == Some(node.id) {
                if isolated.contains(&node.id) {
                    None
                } else {
                    Some(node.id)
                }
            } else {
                None
            }
        })
    }

    /// Restore the network of the specified node.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn restore_node(&self, id: NodeId) {
        let mut nodes = self.isolated_nodes.write().await;
        nodes.remove(&id);
    }

    pub async fn add_non_voter(&self, leader: NodeId, target: NodeId) -> Result<(), ChangeConfigError> {
        let rt = self.routing_table.read().await;
        let node = rt.get(&leader).unwrap_or_else(|| panic!("node with ID {} does not exist", leader));
        node.0.add_non_voter(target).await
    }

    pub async fn change_membership(&self, leader: NodeId, members: HashSet<NodeId>) -> Result<(), ChangeConfigError> {
        let rt = self.routing_table.read().await;
        let node = rt.get(&leader).unwrap_or_else(|| panic!("node with ID {} does not exist", leader));
        node.0.change_membership(members).await
    }

    /// Send a client read request to the target node.
    pub async fn client_read(&self, target: NodeId) -> Result<(), ClientReadError> {
        let rt = self.routing_table.read().await;
        let node = rt.get(&target).unwrap_or_else(|| panic!("node with ID {} does not exist", target));
        node.0.client_read().await
    }

    /// Send a client request to the target node, causing test failure on error.
    pub async fn client_request(&self, target: NodeId, client_id: &str, serial: u64) {
        let req = MemClientRequest {
            client: client_id.into(),
            serial,
            status: format!("request-{}", serial),
        };
        if let Err(err) = self.send_client_request(target, req).await {
            tracing::error!({error=%err}, "error from client request");
            panic!("{:?}", err)
        }
    }

    /// Request the current leader from the target node.
    pub async fn current_leader(&self, target: NodeId) -> Option<NodeId> {
        let rt = self.routing_table.read().await;
        let node = rt.get(&target).unwrap_or_else(|| panic!("node with ID {} does not exist", target));
        node.0.current_leader().await
    }

    /// Send multiple client requests to the target node, causing test failure on error.
    pub async fn client_request_many(&self, target: NodeId, client_id: &str, count: usize) {
        for idx in 0..count {
            self.client_request(target, client_id, idx as u64).await
        }
    }

    async fn send_client_request(
        &self, target: NodeId, req: MemClientRequest,
    ) -> std::result::Result<MemClientResponse, ClientWriteError<MemClientRequest>> {
        let rt = self.routing_table.read().await;
        let node = rt
            .get(&target)
            .unwrap_or_else(|| panic!("node '{}' does not exist in routing table", target));
        node.0.client_write(ClientWriteRequest::new(req)).await.map(|res| res.data)
    }

    //////////////////////////////////////////////////////////////////////////////////////////////
    //////////////////////////////////////////////////////////////////////////////////////////////

    /// Assert that the cluster is in a pristine state, with all nodes as non-voters.
    pub async fn assert_pristine_cluster(&self) {
        let nodes = self.latest_metrics().await;
        for node in nodes.iter() {
            assert!(node.current_leader.is_none(), "node {} has a current leader, expected none", node.id);
            assert_eq!(node.state, State::NonVoter, "node is in state {:?}, expected NonVoter", node.state);
            assert_eq!(node.current_term, 0, "node {} has term {}, expected 0", node.id, node.current_term);
            assert_eq!(
                node.last_applied, 0,
                "node {} has last_applied {}, expected 0",
                node.id, node.last_applied
            );
            assert_eq!(
                node.last_log_index, 0,
                "node {} has last_log_index {}, expected 0",
                node.id, node.last_log_index
            );
            let members = node.membership_config.members.iter().collect::<Vec<_>>();
            assert_eq!(members, vec![&node.id], "node {0} has membership {1:?}, expected [{0}]", node.id, members);
            assert!(
                node.membership_config.members_after_consensus.is_none(),
                "node {} is in joint consensus, expected uniform consensus",
                node.id
            );
        }
    }

    /// Assert that the cluster has an elected leader, and is in a stable state with all nodes uniform.
    ///
    /// If `expected_term` is `Some`, then all nodes will be tested to ensure that they are in the
    /// given term. Else, the leader's current term will be used for the assertion.
    ///
    /// If `expected_last_log` is `Some`, then all nodes will be tested to ensure that their last
    /// log index and last applied log match the given value. Else, the leader's last_log_index
    /// will be used for the assertion.
    pub async fn assert_stable_cluster(&self, expected_term: Option<u64>, expected_last_log: Option<u64>) {
        let isolated = self.isolated_nodes.read().await;
        let nodes = self.latest_metrics().await;

        let non_isolated_nodes: Vec<_> = nodes.iter().filter(|node| !isolated.contains(&node.id)).collect();
        let leader = nodes
            .iter()
            .filter(|node| !isolated.contains(&node.id))
            .find(|node| node.state == State::Leader)
            .expect("expected to find a cluster leader");
        let followers: Vec<_> = nodes
            .iter()
            .filter(|node| !isolated.contains(&node.id))
            .filter(|node| node.state == State::Follower)
            .collect();

        assert_eq!(
            followers.len() + 1,
            non_isolated_nodes.len(),
            "expected all nodes to be followers with one leader, got 1 leader and {} followers, expected {} followers",
            followers.len(),
            non_isolated_nodes.len() - 1,
        );
        let expected_term = match expected_term {
            Some(term) => term,
            None => leader.current_term,
        };
        let expected_last_log = match expected_last_log {
            Some(idx) => idx,
            None => leader.last_log_index,
        };
        let all_nodes = nodes.iter().map(|node| node.id).collect::<Vec<_>>();
        for node in non_isolated_nodes.iter() {
            assert_eq!(
                node.current_leader,
                Some(leader.id),
                "node {} has leader {:?}, expected {}",
                node.id,
                node.current_leader,
                leader.id
            );
            assert_eq!(
                node.current_term, expected_term,
                "node {} has term {}, expected {}",
                node.id, node.current_term, expected_term
            );
            assert_eq!(
                node.last_applied, expected_last_log,
                "node {} has last_applied {}, expected {}",
                node.id, node.last_applied, expected_last_log
            );
            assert_eq!(
                node.last_log_index, expected_last_log,
                "node {} has last_log_index {}, expected {}",
                node.id, node.last_log_index, expected_last_log
            );
            let mut members = node.membership_config.members.iter().cloned().collect::<Vec<_>>();
            members.sort_unstable();
            assert_eq!(
                members, all_nodes,
                "node {} has membership {:?}, expected {:?}",
                node.id, members, all_nodes
            );
            assert!(
                node.membership_config.members_after_consensus.is_none(),
                "node {} was not in uniform consensus state",
                node.id
            );
        }
    }

    /// Assert against the state of the storage system per node in the cluster.
    pub async fn assert_storage_state(
        &self, expect_term: u64, expect_last_log: u64, expect_voted_for: Option<u64>, expect_sm_last_applied_log: u64,
        expect_snapshot: Option<(ValueTest<u64>, u64, MembershipConfig)>,
    ) {
        let rt = self.routing_table.read().await;
        for (id, (_node, storage)) in rt.iter() {
            let log = storage.get_log().await;
            let last_log = log.keys().last().unwrap_or_else(|| panic!("no last log found for node {}", id));
            assert_eq!(
                last_log, &expect_last_log,
                "expected node {} to have last_log {}, got {}",
                id, expect_last_log, last_log
            );
            let hs = storage
                .read_hard_state()
                .await
                .clone()
                .unwrap_or_else(|| panic!("no hardstate found for node {}", id));
            assert_eq!(
                hs.current_term, expect_term,
                "expected node {} to have term {}, got {}",
                id, expect_term, hs.current_term
            );
            if let Some(voted_for) = &expect_voted_for {
                assert_eq!(
                    hs.voted_for.as_ref(),
                    Some(voted_for),
                    "expected node {} to have voted for {}, got {:?}",
                    id,
                    voted_for,
                    hs.voted_for
                );
            }
            if let Some((index_test, term, cfg)) = &expect_snapshot {
                let snap = storage
                    .get_current_snapshot()
                    .await
                    .map_err(|err| panic!("{}", err))
                    .unwrap()
                    .unwrap_or_else(|| panic!("no snapshot present for node {}", id));
                match index_test {
                    ValueTest::Exact(index) => assert_eq!(
                        &snap.index, index,
                        "expected node {} to have snapshot with index {}, got {}",
                        id, index, snap.index
                    ),
                    ValueTest::Range(range) => assert!(
                        range.contains(&snap.index),
                        "expected node {} to have snapshot within range {:?}, got {}",
                        id,
                        range,
                        snap.index
                    ),
                }
                assert_eq!(
                    &snap.term, term,
                    "expected node {} to have snapshot with term {}, got {}",
                    id, term, snap.term
                );
                assert_eq!(
                    &snap.membership, cfg,
                    "expected node {} to have membership config {:?}, got {:?}",
                    id, cfg, snap.membership
                );
            }
            let sm = storage.get_state_machine().await;
            assert_eq!(
                &sm.last_applied_log, &expect_sm_last_applied_log,
                "expected node {} to have state machine last_applied_log {}, got {}",
                id, expect_sm_last_applied_log, sm.last_applied_log
            );
        }
    }
}

#[async_trait]
impl RaftNetwork<MemClientRequest> for RaftRouter {
    /// Send an AppendEntries RPC to the target Raft node (§5).
    async fn append_entries(&self, target: u64, rpc: AppendEntriesRequest<MemClientRequest>) -> Result<AppendEntriesResponse> {
        let rt = self.routing_table.read().await;
        let isolated = self.isolated_nodes.read().await;
        let addr = rt.get(&target).expect("target node not found in routing table");
        if isolated.contains(&target) || isolated.contains(&rpc.leader_id) {
            return Err(anyhow!("target node is isolated"));
        }
        Ok(addr.0.append_entries(rpc).await?)
    }

    /// Send an InstallSnapshot RPC to the target Raft node (§7).
    async fn install_snapshot(&self, target: u64, rpc: InstallSnapshotRequest) -> Result<InstallSnapshotResponse> {
        let rt = self.routing_table.read().await;
        let isolated = self.isolated_nodes.read().await;
        let addr = rt.get(&target).expect("target node not found in routing table");
        if isolated.contains(&target) || isolated.contains(&rpc.leader_id) {
            return Err(anyhow!("target node is isolated"));
        }
        Ok(addr.0.install_snapshot(rpc).await?)
    }

    /// Send a RequestVote RPC to the target Raft node (§5).
    async fn vote(&self, target: u64, rpc: VoteRequest) -> Result<VoteResponse> {
        let rt = self.routing_table.read().await;
        let isolated = self.isolated_nodes.read().await;
        let addr = rt.get(&target).expect("target node not found in routing table");
        if isolated.contains(&target) || isolated.contains(&rpc.candidate_id) {
            return Err(anyhow!("target node is isolated"));
        }
        Ok(addr.0.vote(rpc).await?)
    }
}

pub enum ValueTest<T> {
    Exact(T),
    Range(std::ops::Range<T>),
}

impl<T> From<T> for ValueTest<T> {
    fn from(src: T) -> Self {
        Self::Exact(src)
    }
}

impl<T> From<std::ops::Range<T>> for ValueTest<T> {
    fn from(src: std::ops::Range<T>) -> Self {
        Self::Range(src)
    }
}
