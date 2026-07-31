//! Main EzRaft API
//!
//! This module provides the primary [`EzRaft`] struct that users interact with.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use openraft::BasicNode;
use openraft::ChangeMembers;
use openraft::Raft;
use openraft::ReadPolicy;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::ChangeMembershipError;
use openraft::errors::ClientWriteError;
use openraft::errors::InitializeError;
use openraft::errors::RaftError;
use serde::Serialize;
use tokio::time::sleep;

use crate::app::EzApp;
use crate::config::EzConfig;
use crate::network::EzNetworkFactory;
use crate::storage::EzStorage;
use crate::storage::adapter::StorageAdapter;
use crate::type_config::OpenRaftTypes;

/// Type alias for OpenRaft types (more readable than `OpenRaftTypes<T>`)
type ORTypes<T> = OpenRaftTypes<T>;

/// The internal OpenRaft instance, with EzRaft's storage adapter as its state machine
pub type ORRaft<T> = Raft<ORTypes<T>, Arc<StorageAdapter<T>>>;

/// EzRaft - A simplified Raft interface
///
/// This struct wraps OpenRaft's `Raft` and provides a simplified API.
/// Users create an instance with their app and storage, then call
/// methods to initialize the cluster, write data, and serve HTTP requests.
///
/// # Type Parameters
///
/// - `T`: The application (implements `EzApp`)
pub struct EzRaft<T>
where T: EzApp
{
    /// Node ID
    node_id: u64,

    /// HTTP bind address
    addr: String,

    /// Storage adapter (bridges user storage/state machine to OpenRaft)
    storage: Arc<StorageAdapter<T>>,

    /// Internal OpenRaft instance
    raft: ORRaft<T>,
}

impl<T> Clone for EzRaft<T>
where T: EzApp
{
    fn clone(&self) -> Self {
        Self {
            node_id: self.node_id,
            addr: self.addr.clone(),
            storage: self.storage.clone(),
            raft: self.raft.clone(),
        }
    }
}

impl<T> EzRaft<T>
where T: EzApp
{
    /// Start a new cluster with this node as its only member
    ///
    /// Exactly one node of a cluster is created this way; every other node uses [`Self::join`].
    /// Creating two nodes separately gives two one-node clusters that will never merge.
    ///
    /// # Arguments
    ///
    /// * `http_addr` - Address to bind HTTP server (e.g., "127.0.0.1:8080")
    /// * `app` - User's application (state machine)
    /// * `storage` - User's storage implementation
    /// * `config` - EzRaft configuration (use `EzConfig::default()` for sensible defaults)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let raft = EzRaft::create("127.0.0.1:8080", app, storage, config).await?;
    /// ```
    pub async fn create(
        http_addr: impl ToString,
        app: T,
        storage: impl EzStorage<T>,
        config: EzConfig,
    ) -> Result<Self, io::Error> {
        Self::new(http_addr, app, storage, config, None).await
    }

    /// Join the cluster that `seed_addr` belongs to
    ///
    /// The seed assigns this node an id and adds it to the cluster; the seed does not have to be
    /// the leader. On restart the persisted id is reused and the seed is not contacted again, so
    /// passing an address that has since left the cluster is harmless.
    ///
    /// # Arguments
    ///
    /// * `http_addr` - Address to bind HTTP server (e.g., "127.0.0.1:8081")
    /// * `seed_addr` - Address of any node already in the cluster
    /// * `app` - User's application (state machine)
    /// * `storage` - User's storage implementation
    /// * `config` - EzRaft configuration (use `EzConfig::default()` for sensible defaults)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let raft = EzRaft::join("127.0.0.1:8081", "127.0.0.1:8080", app, storage, config).await?;
    /// ```
    pub async fn join(
        http_addr: impl ToString,
        seed_addr: impl ToString,
        app: T,
        storage: impl EzStorage<T>,
        config: EzConfig,
    ) -> Result<Self, io::Error> {
        Self::new(http_addr, app, storage, config, Some(seed_addr.to_string())).await
    }

    async fn new(
        http_addr: impl ToString,
        app: T,
        storage: impl EzStorage<T>,
        config: EzConfig,
        seed_addr: Option<String>,
    ) -> Result<Self, io::Error> {
        let http_addr = http_addr.to_string();

        // Create storage adapter that bridges user traits to OpenRaft
        let adapter = StorageAdapter::new(storage, app).await?;
        let adapter = Arc::new(adapter);

        // Determine node_id
        let node_id = if let Some(id) = adapter.node_id().await {
            // Use persisted node_id (restart case)
            id
        } else if let Some(seed) = &seed_addr {
            // Join existing cluster via seed node
            let id = request_join(seed, &http_addr).await?;
            adapter.save_meta(|m| m.node_id = Some(id)).await?;
            id
        } else {
            // First node in cluster
            let id = 0;
            adapter.save_meta(|m| m.node_id = Some(id)).await?;
            id
        };

        let (log_store, sm_store) = (adapter.clone(), adapter.clone());

        // Convert EzConfig to OpenRaft Config
        let raft_config = config.to_raft_config()?;
        let raft_config = Arc::new(raft_config);

        // Create network factory
        let network = EzNetworkFactory::new()?;

        // Create OpenRaft instance
        let raft = Raft::new(node_id, raft_config, network, log_store, sm_store)
            .await
            .map_err(|e| io::Error::other(e.to_string()))?;

        // The created node starts the cluster with itself as its only member. On restart it loads
        // id 0 from storage and comes back here, where initializing is rightly refused; every
        // other refusal means the node cannot run.
        if node_id == 0 {
            let nodes = BTreeMap::from_iter([(node_id, BasicNode::new(http_addr.clone()))]);
            match raft.initialize(nodes).await {
                Ok(()) | Err(RaftError::APIError(InitializeError::NotAllowed(_))) => {}
                Err(e) => return Err(io::Error::other(e.to_string())),
            }
        }

        let this = Self {
            node_id,
            addr: http_addr,
            storage: adapter,
            raft,
        };

        // Promotion belongs to whoever currently leads, so every node runs the loop; it acts
        // only while this node is the leader.
        tokio::spawn(this.clone().reconcile_learners());

        Ok(this)
    }

    /// Promote each caught-up learner while this node leads
    ///
    /// EzRaft has no lasting learner state: a learner is a node that has joined and not been
    /// promoted yet. The join handler only records the learner; promotion happens here, owned
    /// by the leader role rather than by the node that handled the join, so a promotion
    /// interrupted by a crash or a leader change is finished by whoever leads next instead of
    /// dying with the task that started it.
    async fn reconcile_learners(self) {
        let promotable = |m: &openraft::RaftMetrics<ORTypes<T>>| -> Option<u64> {
            let replication = m.replication.as_ref()?;
            let membership = m.membership_config.membership();
            membership.learner_ids().find(|id| {
                let matched = replication.get(id).and_then(|log_id| log_id.as_ref()).map(|log_id| log_id.index);
                matched >= m.last_log_index
            })
        };

        loop {
            let res =
                self.raft.wait(None).metrics(|m| promotable(m).is_some(), "a learner is ready for promotion").await;

            let Ok(metrics) = res else {
                // The node is shutting down.
                return;
            };

            let Some(node_id) = promotable(&metrics) else {
                continue;
            };

            if let Err(e) = self.promote_to_voter(node_id).await {
                tracing::error!("failed to promote node {} to voter: {}", node_id, e);
                sleep(PROMOTE_RETRY_INTERVAL).await;
            }
        }
    }

    /// Write a request to the Raft log
    ///
    /// This proposes a client request to the Raft cluster.
    /// The request will be replicated and applied to the state machine once committed.
    ///
    /// Only a leader can accept a write. Calling this on a follower forwards the request to the
    /// leader over HTTP and returns the leader's answer, so a caller never has to track which
    /// node is currently in charge. Moments without a usable leader - an election in flight, a
    /// just-elected leader that has not confirmed its lease - are waited out for up to ten
    /// seconds before the write fails.
    ///
    /// # Arguments
    ///
    /// * `req` - User's request type
    ///
    /// # Returns
    ///
    /// The response from the state machine's `apply()` method
    ///
    /// # Example
    ///
    /// ```ignore
    /// let req = Request::Set { key: "foo".into(), value: "bar".into() };
    /// let resp = raft.write(req).await?;
    /// ```
    pub async fn write(&self, req: T::Request) -> Result<T::Response, io::Error> {
        let mut last_err = String::new();

        for _ in 0..WRITE_ATTEMPTS {
            let err = match self.raft.client_write(req.clone()).await {
                // A user write is always answered with `Some` by `apply`; `None` exists only
                // for framework-generated entries.
                Ok(resp) => return resp.data.ok_or_else(|| io::Error::other("write produced no response")),
                Err(e) => e,
            };

            let RaftError::APIError(ClientWriteError::ForwardToLeader(forward)) = &err else {
                return Err(io::Error::other(err.to_string()));
            };

            // Forwarding to ourselves would repeat this call over HTTP forever.
            match forward.leader_node.as_ref().map(|n| n.addr.as_str()) {
                Some(leader) if leader != self.addr => return forward_write::<T>(leader, &req).await,
                // No usable leader: an election is in flight, or a just-elected leader has not
                // confirmed its quorum lease yet. Both resolve within heartbeats, so wait them
                // out instead of bothering the caller.
                _ => {
                    last_err = err.to_string();
                    sleep(WRITE_RETRY_INTERVAL).await;
                }
            }
        }

        Err(io::Error::other(format!(
            "write gave up after {} attempts: {}",
            WRITE_ATTEMPTS, last_err
        )))
    }

    /// Read the applied state directly, without going through the log
    ///
    /// Runs the closure over this node's applied application state and returns its result. The
    /// read is local and cheap - no consensus round, no log entry - and therefore not
    /// linearizable on its own: this node, leader included, may not have applied the latest
    /// acknowledged write yet. Call [`Self::linearizable`] first when read-your-writes matters.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let value = raft.read(|app| app.data.get("foo").cloned()).await;
    /// ```
    pub async fn read<F, R>(&self, read: F) -> R
    where F: FnOnce(&T) -> R {
        let sm = self.storage.sm_state.lock().await;
        read(&sm.app)
    }

    /// Wait until a local read would be linearizable
    ///
    /// Confirms this node is still the leader with a quorum round-trip, then waits until the
    /// local state machine has applied everything committed up to that point. A [`Self::read`]
    /// issued after this returns sees every write acknowledged before this call.
    ///
    /// Only the leader can serve linearizable reads, so on a follower this returns an error
    /// instead of forwarding: forwarding cannot make this node's local state current.
    pub async fn linearizable(&self) -> Result<(), io::Error> {
        self.raft
            .ensure_linearizable(ReadPolicy::ReadIndex)
            .await
            .map_err(|e| io::Error::other(e.to_string()))?;
        Ok(())
    }

    /// Add a learner node to the cluster
    ///
    /// A learner receives log replication but does not vote. In EzRaft a learner is always a
    /// transient state - the first half of admitting a node, not a way to build read-only
    /// replicas: [`Self::reconcile_learners`] promotes every learner once it has caught up.
    ///
    /// Returns as soon as replication to the new node is set up; the node catches up in the
    /// background. Waiting here would deadlock the join handler, whose caller cannot answer any
    /// Raft RPC until it gets its node id back.
    ///
    /// # Arguments
    ///
    /// * `node_id` - ID of the new learner node
    /// * `addr` - Address of the new learner node
    pub(crate) async fn add_learner(&self, node_id: u64, addr: String) -> Result<(), io::Error> {
        let node = BasicNode::new(addr);
        self.raft.add_learner(node_id, node, false).await.map_err(|e| io::Error::other(e.to_string()))?;

        Ok(())
    }

    /// Wait for a learner to catch up, then make it a voter
    ///
    /// Only voters count towards a quorum, so a cluster tolerates a node failure only once its
    /// nodes have been promoted. Promoting a node that is still far behind would stall the
    /// membership change, hence the wait.
    ///
    /// A cluster admits one membership change at a time, so when several nodes join at once
    /// their promotions take turns: one that finds another change in flight waits and retries.
    ///
    /// Returns without changing anything if the node is already a voter, or if this node is no
    /// longer the leader - the new leader's [`Self::reconcile_learners`] owns the promotion
    /// from that point on.
    async fn promote_to_voter(&self, node_id: u64) -> Result<(), io::Error> {
        let caught_up = |m: &openraft::RaftMetrics<ORTypes<T>>| {
            let Some(replication) = m.replication.as_ref() else {
                // Not the leader anymore, stop waiting.
                return true;
            };
            let matched = replication.get(&node_id).and_then(|log_id| log_id.as_ref()).map(|log_id| log_id.index);
            matched >= m.last_log_index
        };

        let mut last_err = String::new();

        for _ in 0..PROMOTE_ATTEMPTS {
            let metrics = self
                .raft
                .wait(None)
                .metrics(caught_up, "learner catches up before promotion")
                .await
                .map_err(|e| io::Error::other(e.to_string()))?;

            if metrics.current_leader != Some(self.node_id) {
                return Ok(());
            }

            if metrics.membership_config.membership().voter_ids().any(|id| id == node_id) {
                return Ok(());
            }

            let change = ChangeMembers::AddVoterIds(BTreeSet::from([node_id]));
            let err = match self.raft.change_membership(change, false).await {
                Ok(_) => return Ok(()),
                Err(e) => e,
            };

            match &err {
                RaftError::APIError(ClientWriteError::ChangeMembershipError(ChangeMembershipError::InProgress(_))) => {
                    last_err = err.to_string();
                    sleep(PROMOTE_RETRY_INTERVAL).await;
                }
                // Deposed between the check above and the change; the new leader owns it now.
                RaftError::APIError(ClientWriteError::ForwardToLeader(_)) => return Ok(()),
                _ => return Err(io::Error::other(err.to_string())),
            }
        }

        Err(io::Error::other(format!(
            "promotion of node {} gave up after {} attempts: {}",
            node_id, PROMOTE_ATTEMPTS, last_err
        )))
    }

    /// Change the cluster membership
    ///
    /// This modifies the cluster membership using OpenRaft's `ChangeMembers`.
    pub async fn change_membership(&self, change: ChangeMembers<u64, BasicNode>) -> Result<(), io::Error> {
        self.raft.change_membership(change, false).await.map_err(|e| io::Error::other(e.to_string()))?;
        Ok(())
    }

    /// Check if this node is the leader
    ///
    /// Reports this node's own state, which is what a caller asking "am I the leader" wants. It
    /// is not a guarantee that the answer is still true elsewhere: a deposed leader does not
    /// find out until it hears from the new one. Nothing here needs that guarantee --
    /// [`Self::write`] finds the leader on its own -- and code that does want a linearizable
    /// read calls [`Self::linearizable`] first.
    pub fn is_leader(&self) -> bool {
        self.raft.is_leader()
    }

    /// Get the current cluster metrics
    ///
    /// Returns information about the Raft cluster state.
    pub async fn metrics(&self) -> openraft::RaftMetrics<ORTypes<T>> {
        self.raft.metrics().borrow_watched().clone()
    }

    /// Start the HTTP server
    ///
    /// This starts the HTTP server that handles:
    /// - Internal Raft RPC (append entries, vote, install snapshot)
    /// - Admin API (join, add learner, change membership, metrics)
    ///
    /// This method blocks until the server is stopped, so a caller with other work to do spawns
    /// it: `tokio::spawn(raft.clone().serve())`. Start it as early as possible. Peers reach a
    /// node only through this server, so a node that has joined a cluster but is not serving yet
    /// cannot be replicated to, and holds up every quorum it is counted in.
    pub async fn serve(self) -> Result<(), io::Error> {
        crate::server::run(self).await
    }

    /// Get the node ID
    pub fn node_id(&self) -> u64 {
        self.node_id
    }

    /// Get the HTTP address
    pub fn addr(&self) -> &str {
        &self.addr
    }

    /// Get a reference to the internal OpenRaft instance
    ///
    /// This provides access to advanced OpenRaft APIs if needed.
    pub fn inner(&self) -> &ORRaft<T> {
        &self.raft
    }
}

/// How many times a write retries while the cluster has no usable leader
const WRITE_ATTEMPTS: usize = 20;

/// How long to wait before retrying a write
const WRITE_RETRY_INTERVAL: Duration = Duration::from_millis(500);

/// How many times a promotion retries while another membership change is in flight
const PROMOTE_ATTEMPTS: usize = 20;

/// How long to wait before retrying a promotion
const PROMOTE_RETRY_INTERVAL: Duration = Duration::from_millis(500);

/// How long a forwarded write may take before the leader is given up on
///
/// Generous, because the leader has to replicate and commit the request before answering.
const FORWARD_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Send a write to the leader's `/api/write` endpoint and return what it applied
///
/// The leader is asked to do the write on this node's behalf, so the answer is the same one the
/// caller would have got from writing to the leader directly.
async fn forward_write<T>(leader_addr: &str, req: &T::Request) -> Result<T::Response, io::Error>
where T: EzApp {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(FORWARD_WRITE_TIMEOUT)
        .build()
        .map_err(|e| io::Error::other(e.to_string()))?;

    let url = format!("http://{}/api/write", leader_addr);

    let resp = client
        .post(&url)
        .json(req)
        .send()
        .await
        .map_err(|e| io::Error::other(format!("forwarding write to {} failed: {}", url, e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(io::Error::other(format!("{} responded {}: {}", url, status, body)));
    }

    resp.json().await.map_err(|e| io::Error::other(format!("failed to parse write response: {}", e)))
}

/// Request to join a cluster
#[derive(Debug, Serialize)]
struct JoinRequest {
    addr: String,
}

/// Join response: Ok(node_id) or Err(leader_addr)
type JoinResponse = Result<u64, Option<String>>;

/// How many times a join is attempted before the node gives up and reports the last failure
const JOIN_ATTEMPTS: usize = 20;

/// How long to wait before attempting a join again
const JOIN_RETRY_INTERVAL: Duration = Duration::from_millis(500);

/// How long a single join request may take before the target is given up on
const JOIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Request to join a cluster via seed node
///
/// Follows the seed's redirect if it is not the leader, and retries the transient conditions a
/// starting cluster is full of: no leader elected yet, or another node's membership change still
/// in flight. A cluster admits one member at a time, so nodes started together take turns here
/// instead of failing.
async fn request_join(seed_addr: &str, my_addr: &str) -> Result<u64, io::Error> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(JOIN_TIMEOUT)
        .build()
        .map_err(|e| io::Error::other(e.to_string()))?;

    let mut target_addr = seed_addr.to_string();
    let mut last_err = "cluster did not accept the join".to_string();

    for _ in 0..JOIN_ATTEMPTS {
        let url = format!("http://{}/api/join", target_addr);
        let req = JoinRequest {
            addr: my_addr.to_string(),
        };

        // A send failure is as transient as the rest: the seed may still be binding its HTTP
        // socket, since serving starts concurrently with cluster formation.
        let resp = match client.post(&url).json(&req).send().await {
            Ok(resp) => resp,
            Err(e) => {
                last_err = format!("join request to {} failed: {}", url, e);
                sleep(JOIN_RETRY_INTERVAL).await;
                continue;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            last_err = format!("{} responded {}: {}", url, status, body);
            sleep(JOIN_RETRY_INTERVAL).await;
            continue;
        }

        let join_resp: JoinResponse =
            resp.json().await.map_err(|e| io::Error::other(format!("failed to parse join response: {}", e)))?;

        match join_resp {
            Ok(node_id) => return Ok(node_id),
            Err(Some(leader)) => {
                last_err = format!("{} redirected to {}", url, leader);
                target_addr = leader;
            }
            Err(None) => {
                last_err = format!("{} knows of no leader", url);
                sleep(JOIN_RETRY_INTERVAL).await;
            }
        }
    }

    Err(io::Error::other(format!(
        "join gave up after {} attempts: {}",
        JOIN_ATTEMPTS, last_err
    )))
}
