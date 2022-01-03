//! Public Raft interface and data types.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::Span;

use crate::config::Config;
use crate::core::RaftCore;
use crate::error::AddLearnerError;
use crate::error::ClientReadError;
use crate::error::ClientWriteError;
use crate::error::InitializeError;
use crate::error::RaftError;
use crate::error::RaftResult;
use crate::metrics::RaftMetrics;
use crate::metrics::Wait;
use crate::quorum;
use crate::AppData;
use crate::AppDataResponse;
use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;
use crate::RaftNetwork;
use crate::RaftStorage;
use crate::SnapshotMeta;

struct RaftInner<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    tx_api: mpsc::UnboundedSender<(RaftMsg<D, R>, Span)>,
    rx_metrics: watch::Receiver<RaftMetrics>,
    raft_handle: Mutex<Option<JoinHandle<RaftResult<()>>>>,
    tx_shutdown: Mutex<Option<oneshot::Sender<()>>>,
    marker_n: std::marker::PhantomData<N>,
    marker_s: std::marker::PhantomData<S>,
}

/// The Raft API.
///
/// This type implements the full Raft spec, and is the interface to a running Raft node.
/// Applications building on top of Raft will use this to spawn a Raft task and interact with
/// the spawned task.
///
/// For more information on the Raft protocol, see
/// [the specification here](https://raft.github.io/raft.pdf) (**pdf warning**).
///
/// For details and discussion on this API, see the
/// [Raft API](https://datafuselabs.github.io/openraft/raft.html) section of the guide.
///
/// ### clone
/// This type implements `Clone`, and should be cloned liberally. The clone itself is very cheap
/// and helps to facilitate use with async workflows.
///
/// ### shutting down
/// If any of the interfaces returns a `RaftError::ShuttingDown`, this indicates that the Raft node
/// is shutting down (potentially for data safety reasons due to a storage error), and the `shutdown`
/// method should be called on this type to await the shutdown of the node. If the parent
/// application needs to shutdown the Raft node for any reason, calling `shutdown` will do the trick.
pub struct Raft<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    inner: Arc<RaftInner<D, R, N, S>>,
}

impl<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> Raft<D, R, N, S> {
    /// Create and spawn a new Raft task.
    ///
    /// ### `id`
    /// The ID which the spawned Raft task will use to identify itself within the cluster.
    /// Applications must guarantee that the ID provided to this function is stable, and should be
    /// persisted in a well known location, probably alongside the Raft log and the application's
    /// state machine. This ensures that restarts of the node will yield the same ID every time.
    ///
    /// ### `config`
    /// Raft's runtime config. See the docs on the `Config` object for more details.
    ///
    /// ### `network`
    /// An implementation of the `RaftNetwork` trait which will be used by Raft for sending RPCs to
    /// peer nodes within the cluster. See the docs on the `RaftNetwork` trait for more details.
    ///
    /// ### `storage`
    /// An implementation of the `RaftStorage` trait which will be used by Raft for data storage.
    /// See the docs on the `RaftStorage` trait for more details.
    #[tracing::instrument(level="debug", skip(config, network, storage), fields(cluster=%config.cluster_name))]
    pub fn new(id: NodeId, config: Arc<Config>, network: Arc<N>, storage: Arc<S>) -> Self {
        let (tx_api, rx_api) = mpsc::unbounded_channel();
        let (tx_metrics, rx_metrics) = watch::channel(RaftMetrics::new_initial(id));
        let (tx_shutdown, rx_shutdown) = oneshot::channel();
        let raft_handle = RaftCore::spawn(id, config, network, storage, rx_api, tx_metrics, rx_shutdown);
        let inner = RaftInner {
            tx_api,
            rx_metrics,
            raft_handle: Mutex::new(Some(raft_handle)),
            tx_shutdown: Mutex::new(Some(tx_shutdown)),
            marker_n: std::marker::PhantomData,
            marker_s: std::marker::PhantomData,
        };
        Self { inner: Arc::new(inner) }
    }

    /// Submit an AppendEntries RPC to this Raft node.
    ///
    /// These RPCs are sent by the cluster leader to replicate log entries (§5.3), and are also
    /// used as heartbeats (§5.2).
    #[tracing::instrument(level = "trace", skip(self, rpc), fields(rpc=%rpc.summary()))]
    pub async fn append_entries(&self, rpc: AppendEntriesRequest<D>) -> Result<AppendEntriesResponse, RaftError> {
        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::AppendEntries { rpc, tx }, rx).await
    }

    /// Submit a VoteRequest (RequestVote in the spec) RPC to this Raft node.
    ///
    /// These RPCs are sent by cluster peers which are in candidate state attempting to gather votes (§5.2).
    #[tracing::instrument(level = "debug", skip(self, rpc), fields(rpc=%rpc.summary()))]
    pub async fn vote(&self, rpc: VoteRequest) -> Result<VoteResponse, RaftError> {
        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::RequestVote { rpc, tx }, rx).await
    }

    /// Submit an InstallSnapshot RPC to this Raft node.
    ///
    /// These RPCs are sent by the cluster leader in order to bring a new node or a slow node up-to-speed
    /// with the leader (§7).
    #[tracing::instrument(level = "debug", skip(self, rpc), fields(snapshot_id=%rpc.meta.last_log_id))]
    pub async fn install_snapshot(&self, rpc: InstallSnapshotRequest) -> Result<InstallSnapshotResponse, RaftError> {
        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::InstallSnapshot { rpc, tx }, rx).await
    }

    /// Get the ID of the current leader from this Raft node.
    ///
    /// This method is based on the Raft metrics system which does a good job at staying
    /// up-to-date; however, the `client_read` method must still be used to guard against stale
    /// reads. This method is perfect for making decisions on where to route client requests.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn current_leader(&self) -> Option<NodeId> {
        self.metrics().borrow().current_leader
    }

    /// Check to ensure this node is still the cluster leader, in order to guard against stale reads (§8).
    ///
    /// The actual read operation itself is up to the application, this method just ensures that
    /// the read will not be stale.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn client_read(&self) -> Result<(), ClientReadError> {
        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::ClientReadRequest { tx }, rx).await
    }

    /// Submit a mutating client request to Raft to update the state of the system (§5.1).
    ///
    /// It will be appended to the log, committed to the cluster, and then applied to the
    /// application state machine. The result of applying the request to the state machine will
    /// be returned as the response from this method.
    ///
    /// Our goal for Raft is to implement linearizable semantics. If the leader crashes after committing
    /// a log entry but before responding to the client, the client may retry the command with a new
    /// leader, causing it to be executed a second time. As such, clients should assign unique serial
    /// numbers to every command. Then, the state machine should track the latest serial number
    /// processed for each client, along with the associated response. If it receives a command whose
    /// serial number has already been executed, it responds immediately without re-executing the
    /// request (§8). The `RaftStorage::apply_entry_to_state_machine` method is the perfect place
    /// to implement this.
    ///
    /// These are application specific requirements, and must be implemented by the application which is
    /// being built on top of Raft.
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub async fn client_write(&self, rpc: ClientWriteRequest<D>) -> Result<ClientWriteResponse<R>, ClientWriteError> {
        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::ClientWriteRequest { rpc, tx }, rx).await
    }

    /// Initialize a pristine Raft node with the given config.
    ///
    /// This command should be called on pristine nodes — where the log index is 0 and the node is
    /// in Learner state — as if either of those constraints are false, it indicates that the
    /// cluster is already formed and in motion. If `InitializeError::NotAllowed` is returned
    /// from this function, it is safe to ignore, as it simply indicates that the cluster is
    /// already up and running, which is ultimately the goal of this function.
    ///
    /// This command will work for single-node or multi-node cluster formation. This command
    /// should be called with all discovered nodes which need to be part of cluster, and as such
    /// it is recommended that applications be configured with an initial cluster formation delay
    /// which will allow time for the initial members of the cluster to be discovered (by the
    /// parent application) for this call.
    ///
    /// If successful, this routine will set the given config as the active config, only in memory,
    /// and will start an election.
    ///
    /// It is recommended that applications call this function based on an initial call to
    /// `RaftStorage.get_initial_state`. If the initial state indicates that the hard state's
    /// current term is `0` and the `last_log_index` is `0`, then this routine should be called
    /// in order to initialize the cluster.
    ///
    /// Once a node becomes leader and detects that its index is 0, it will commit a new config
    /// entry (instead of the normal blank entry created by new leaders).
    ///
    /// Every member of the cluster should perform these actions. This routine is race-condition
    /// free, and Raft guarantees that the first node to become the cluster leader will propagate
    /// only its own config.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn initialize(&self, members: BTreeSet<NodeId>) -> Result<(), InitializeError> {
        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::Initialize { members, tx }, rx).await
    }

    /// Synchronize a new Raft node, optionally, blocking until up-to-speed (§6).
    ///
    /// - Add a node as learner into the cluster.
    /// - Setup replication from leader to it.
    ///
    /// If blocking is true, this function blocks until the leader believes the logs on the new node is up to date,
    /// i.e., ready to join the cluster, as a voter, by calling `change_membership`.
    /// When finished, it returns the last log id on the new node, in a `RaftResponse::LogId`.
    ///
    /// If blocking is false, this function returns at once as successfully setting up the replication.
    ///
    /// If the node to add is already a voter or learner, it returns `RaftResponse::NoChange` at once.
    #[tracing::instrument(level = "debug", skip(self, id), fields(target=id))]
    pub async fn add_learner(&self, id: NodeId, blocking: bool) -> Result<AddLearnerResponse, AddLearnerError> {
        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::AddLearner { id, blocking, tx }, rx).await
    }

    /// Propose a cluster configuration change.
    ///
    /// If a node in the proposed config but is not yet a voter or learner, it first calls `add_learner` to setup
    /// replication to the new node.
    ///
    /// Internal:
    /// - It proposes a **joint** config.
    /// - When the **joint** config is committed, it proposes a uniform config.
    ///
    /// If blocking is true, it blocks until every learner becomes up to date.
    /// Otherwise it returns error `ChangeMembershipError::LearnerIsLagging` if there is a lagging learner.
    ///
    /// If it lost leadership or crashed before committing the second **uniform** config log, the cluster is left in the
    /// **joint** config.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn change_membership(
        &self,
        members: BTreeSet<NodeId>,
        blocking: bool,
    ) -> Result<ClientWriteResponse<R>, ClientWriteError> {
        tracing::info!(?members, "change_membership: add every member as learner");

        for id in members.iter() {
            let res = self.add_learner(*id, blocking).await;
            let res_err = match res {
                Ok(_) => {
                    continue;
                }
                Err(e) => e,
            };

            tracing::info!(%res_err, "add learner: already exists");

            match res_err {
                AddLearnerError::RaftError(raft_err) => {
                    return Err(ClientWriteError::RaftError(raft_err));
                }
                // TODO(xp): test add learner on non-leader
                AddLearnerError::ForwardToLeader(forward_err) => {
                    return Err(ClientWriteError::ForwardToLeader(forward_err))
                }
                AddLearnerError::Exists(node_id) => {
                    tracing::info!(%node_id, "add learner: already exists");
                    continue;
                }
            }
        }

        tracing::info!("change_membership: start to commit joint config");

        let (tx, rx) = oneshot::channel();
        // res is error if membership can not be changed.
        // If it is not error, it will go into a joint state
        let res = self
            .call_core(
                RaftMsg::ChangeMembership {
                    members: members.clone(),
                    blocking,
                    tx,
                },
                rx,
            )
            .await?;

        tracing::info!("res of first change_membership: {:?}", res.summary());

        let (log_id, joint) = (res.log_id, res.membership.clone().unwrap());

        // There is a previously in progress joint state and it becomes the membership config we want.
        if !joint.is_in_joint_consensus() {
            return Ok(res);
        }

        tracing::debug!("committed a joint config: {} {:?}", log_id, joint);
        tracing::debug!("the second step is to change to uniform config: {:?}", members);

        let (tx, rx) = oneshot::channel();
        let res = self.call_core(RaftMsg::ChangeMembership { members, blocking, tx }, rx).await?;

        tracing::info!("res of second change_membership: {}", res.summary());

        Ok(res)
    }

    /// Invoke RaftCore by sending a RaftMsg and blocks waiting for response.
    #[tracing::instrument(level = "debug", skip(self, mes, rx))]
    pub(crate) async fn call_core<T, E>(&self, mes: RaftMsg<D, R>, rx: RaftRespRx<T, E>) -> Result<T, E>
    where E: From<RaftError> {
        let span = tracing::Span::current();

        let sum = mes.summary();

        let send_res = self.inner.tx_api.send((mes, span));
        if let Err(send_err) = send_res {
            tracing::error!(%send_err, mes=%sum, "error send tx to RaftCore");
            return Err(RaftError::ShuttingDown.into());
        }

        let recv_res = rx.await;
        let res = match recv_res {
            Ok(x) => x,
            Err(e) => {
                tracing::error!(%e, mes=%sum, "error recv rx from RaftCore");
                Err(RaftError::ShuttingDown.into())
            }
        };

        res
    }

    /// Get a handle to the metrics channel.
    pub fn metrics(&self) -> watch::Receiver<RaftMetrics> {
        self.inner.rx_metrics.clone()
    }

    /// Get a handle to wait for the metrics to satisfy some condition.
    ///
    /// ```ignore
    /// # use std::time::Duration;
    /// # use openraft::{State, Raft};
    ///
    /// let timeout = Duration::from_millis(200);
    ///
    /// // wait for raft log-3 to be received and applied:
    /// r.wait(Some(timeout)).log(3).await?;
    ///
    /// // wait for ever for raft node's current leader to become 3:
    /// r.wait(None).current_leader(2).await?;
    ///
    /// // wait for raft state to become a follower
    /// r.wait(None).state(State::Follower).await?;
    /// ```
    pub fn wait(&self, timeout: Option<Duration>) -> Wait {
        let timeout = match timeout {
            Some(t) => t,
            None => Duration::from_millis(500),
        };
        Wait {
            timeout,
            rx: self.inner.rx_metrics.clone(),
        }
    }

    /// Shutdown this Raft node.
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        if let Some(tx) = self.inner.tx_shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.inner.raft_handle.lock().await.take() {
            let _ = handle.await?;
        }
        Ok(())
    }
}

impl<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> Clone for Raft<D, R, N, S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

pub(crate) type RaftRespTx<T, E> = oneshot::Sender<Result<T, E>>;
pub(crate) type RaftRespRx<T, E> = oneshot::Receiver<Result<T, E>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddLearnerResponse {
    pub matched: LogId,
}

/// A message coming from the Raft API.
pub(crate) enum RaftMsg<D: AppData, R: AppDataResponse> {
    AppendEntries {
        rpc: AppendEntriesRequest<D>,
        tx: RaftRespTx<AppendEntriesResponse, RaftError>,
    },
    RequestVote {
        rpc: VoteRequest,
        tx: RaftRespTx<VoteResponse, RaftError>,
    },
    InstallSnapshot {
        rpc: InstallSnapshotRequest,
        tx: RaftRespTx<InstallSnapshotResponse, RaftError>,
    },
    ClientWriteRequest {
        rpc: ClientWriteRequest<D>,
        tx: RaftRespTx<ClientWriteResponse<R>, ClientWriteError>,
    },
    ClientReadRequest {
        tx: RaftRespTx<(), ClientReadError>,
    },
    Initialize {
        members: BTreeSet<NodeId>,
        tx: RaftRespTx<(), InitializeError>,
    },
    // TODO(xp): make tx a field of a struct
    /// Request raft core to setup a new replication to a learner.
    AddLearner {
        id: NodeId,

        /// If block until the newly added learner becomes line-rate.
        blocking: bool,

        /// Send the log id when the replication becomes line-rate.
        tx: RaftRespTx<AddLearnerResponse, AddLearnerError>,
    },
    ChangeMembership {
        members: BTreeSet<NodeId>,
        /// with blocking==false, respond to client a ChangeMembershipError::LearnerIsLagging error at once if a
        /// non-member is lagging.
        ///
        /// Otherwise, wait for commit of the member change log.
        blocking: bool,
        tx: RaftRespTx<ClientWriteResponse<R>, ClientWriteError>,
    },
}

impl<D, R> MessageSummary for RaftMsg<D, R>
where
    D: AppData,
    R: AppDataResponse,
{
    fn summary(&self) -> String {
        match self {
            RaftMsg::AppendEntries { rpc, .. } => {
                format!("AppendEntries: {}", rpc.summary())
            }
            RaftMsg::RequestVote { rpc, .. } => {
                format!("RequestVote: {}", rpc.summary())
            }
            RaftMsg::InstallSnapshot { rpc, .. } => {
                format!("InstallSnapshot: {}", rpc.summary())
            }
            RaftMsg::ClientWriteRequest { rpc, .. } => {
                format!("ClientWriteRequest: {}", rpc.summary())
            }
            RaftMsg::ClientReadRequest { .. } => "ClientReadRequest".to_string(),
            RaftMsg::Initialize { members, .. } => {
                format!("Initialize: {:?}", members)
            }
            RaftMsg::AddLearner { id, blocking, .. } => {
                format!("AddLearner: id: {}, blocking: {}", id, blocking)
            }
            RaftMsg::ChangeMembership { members, blocking, .. } => {
                format!("ChangeMembership: members: {:?}, blocking: {}", members, blocking)
            }
        }
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////

/// An RPC sent by a cluster leader to replicate log entries (§5.3), and as a heartbeat (§5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendEntriesRequest<D: AppData> {
    /// The leader's current term.
    pub term: u64,

    /// The leader's ID. Useful in redirecting clients.
    pub leader_id: u64,

    pub prev_log_id: LogId,

    /// The new log entries to store.
    ///
    /// This may be empty when the leader is sending heartbeats. Entries
    /// are batched for efficiency.
    #[serde(bound = "D: AppData")]
    pub entries: Vec<Entry<D>>,

    /// The leader's committed log id.
    pub leader_commit: LogId,
}

impl<D: AppData> MessageSummary for AppendEntriesRequest<D> {
    fn summary(&self) -> String {
        format!(
            "leader={}-{}, prev_log_id={}, leader_commit={}, entries={}",
            self.term,
            self.leader_id,
            self.prev_log_id,
            self.leader_commit,
            self.entries.as_slice().summary()
        )
    }
}

/// The response to an `AppendEntriesRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct AppendEntriesResponse {
    /// The responding node's current term, for leader to update itself.
    pub term: u64,

    /// The last matching log id on follower.
    ///
    /// It is a successful append-entry iff `matched` is `Some()`.
    pub matched: Option<LogId>,

    /// The log id that is different from the leader on follower.
    ///
    /// `conflict` is None if `matched` is `Some()`, because if there is a matching entry, all following inconsistent
    /// entries will be deleted.
    pub conflict: Option<LogId>,
}

impl AppendEntriesResponse {
    pub fn success(&self) -> bool {
        self.matched.is_some()
    }
}

/// A Raft log entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry<D: AppData> {
    pub log_id: LogId,

    /// This entry's payload.
    #[serde(bound = "D: AppData")]
    pub payload: EntryPayload<D>,
}

impl<D: AppData> MessageSummary for Entry<D> {
    fn summary(&self) -> String {
        format!("{}:{}", self.log_id, self.payload.summary())
    }
}

impl<D: AppData> MessageSummary for Option<Entry<D>> {
    fn summary(&self) -> String {
        match self {
            None => "None".to_string(),
            Some(x) => format!("Some({})", x.summary()),
        }
    }
}

impl<D: AppData> MessageSummary for &[Entry<D>] {
    fn summary(&self) -> String {
        let entry_refs: Vec<_> = self.iter().collect();
        entry_refs.as_slice().summary()
    }
}

impl<D: AppData> MessageSummary for &[&Entry<D>] {
    fn summary(&self) -> String {
        let mut res = Vec::with_capacity(self.len());
        if self.len() <= 5 {
            for x in self.iter() {
                let e = format!("{}:{}", x.log_id, x.payload.summary());
                res.push(e);
            }

            res.join(",")
        } else {
            let first = *self.first().unwrap();
            let last = *self.last().unwrap();

            format!("{} ... {}", first.summary(), last.summary())
        }
    }
}

/// Log entry payload variants.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EntryPayload<D: AppData> {
    /// An empty payload committed by a new cluster leader.
    Blank,

    #[serde(bound = "D: AppData")]
    Normal(D),

    /// A change-membership log entry.
    Membership(Membership),
}

impl<D: AppData> MessageSummary for EntryPayload<D> {
    fn summary(&self) -> String {
        match self {
            EntryPayload::Blank => "blank".to_string(),
            EntryPayload::Normal(_n) => "normal".to_string(),
            EntryPayload::Membership(c) => {
                format!("config-change: {:?}", c)
            }
        }
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////

/// The membership configuration of the cluster.
///
/// It could be a joint of one, two or more members list, i.e., a quorum requires a majority of every members.
///
/// The structure of membership is actually a log:
/// ```text
/// 2-3: [6,7,8]
/// 1-2: [3,4,5]
/// 1-1: [1,2,3]
/// ```
///
/// Without any limitation, a node uses the **joint** of every config
/// as the effective quorum.
///
/// But **raft** tries to eliminate the items in a membership config to at most 2, e.g.:
/// single item config is the normal majority quorum,
/// double items config is the raft joint membership config.
///
/// To achieve this, raft has to guarantee that a 2-entries config contains all valid quorum:
/// E.g.: given the current config of node p and q as the following:
///
/// Node p:
/// ```text
/// A-B: [a,b,c]
/// 1-2: [3,4,5] <- commit_index
/// 1-1: [1,2,3]
/// ```
///
/// Node q:
/// ```text
/// X-Y: [x,y,z]
/// 1-1: [1,2,3] <- commit_index
/// ```
///
/// ```text
/// A-B <- p
///  |
/// 1-2   X-Y <- q
///  |  /
/// 1-1
/// ```
///
/// If we knows about which log entry is committed,
/// the effective membership can be reduced to the joint of the last committed membership and all uncommitted
/// memberships, because:
///
/// - Two nodes has equal greatest committed membership always include the last committed membership in the joint config
///   so that they won't both become a leader.
///
/// - A node has smaller committed membership will see a higher log thus it won't be a new leader, such as q.
///
/// This way, to keep at most 2 member list in the membership config:
///
/// - raft does not allow two uncommitted membership in its log,
/// - and stores the last committed membership and the newly proposed membership in on log entry(because raft does not
///   store committed index), which is the joint membership entry.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    /// Multi configs.
    configs: Vec<BTreeSet<NodeId>>,

    /// Cache of all node ids.
    all_nodes: BTreeSet<NodeId>,
}

impl MessageSummary for Membership {
    fn summary(&self) -> String {
        let mut res = vec!["[".to_string()];
        for (i, c) in self.configs.iter().enumerate() {
            if i > 0 {
                res.push(",".to_string());
            }
            res.push(format!("{:?}", c));
        }
        res.push("]".to_string());
        res.join("")
    }
}

impl Membership {
    pub fn new_single(members: BTreeSet<NodeId>) -> Self {
        let configs = vec![members];
        let all_nodes = Self::build_all_nodes(&configs);
        Membership { configs, all_nodes }
    }

    pub fn new_multi(configs: Vec<BTreeSet<NodeId>>) -> Self {
        let all_nodes = Self::build_all_nodes(&configs);
        Membership { configs, all_nodes }
    }

    pub fn all_nodes(&self) -> &BTreeSet<NodeId> {
        &self.all_nodes
    }

    pub fn replace(&mut self, new_configs: Vec<BTreeSet<NodeId>>) {
        self.configs = new_configs;
        self.all_nodes = Self::build_all_nodes(&self.configs);
    }

    pub fn push(&mut self, new_config: BTreeSet<NodeId>) {
        self.configs.push(new_config);
        self.all_nodes = Self::build_all_nodes(&self.configs);
    }

    pub fn get_configs(&self) -> &Vec<BTreeSet<NodeId>> {
        &self.configs
    }

    pub fn get_ith_config(&self, i: usize) -> Option<&BTreeSet<NodeId>> {
        self.configs.get(i)
    }

    // TODO(xp): remove this
    pub fn ith_config(&self, i: usize) -> Vec<NodeId> {
        self.configs[i].iter().cloned().collect()
    }

    /// Check if the given NodeId exists in this membership config.
    pub fn contains(&self, x: &NodeId) -> bool {
        for c in self.configs.iter() {
            if c.contains(x) {
                return true;
            }
        }
        false
    }

    /// Check to see if the config is currently in joint consensus.
    pub fn is_in_joint_consensus(&self) -> bool {
        self.configs.len() > 1
    }

    // TODO(xp): rename this
    /// Create a new initial config containing only the given node ID.
    pub fn new_initial(id: NodeId) -> Self {
        Membership::new_single(btreeset! {id})
    }

    #[must_use]
    pub fn to_final_config(&self) -> Self {
        assert!(!self.configs.is_empty());

        let last = self.configs.last().cloned().unwrap();
        Membership::new_single(last)
    }

    /// Return true if the given set of ids constitutes a majority.
    ///
    /// I.e. the id set includes a majority of every config.
    pub fn is_majority(&self, granted: &BTreeSet<NodeId>) -> bool {
        for config in self.configs.iter() {
            if !Self::is_majority_of_single_config(granted, config) {
                return false;
            }
        }

        true
    }

    /// Returns the greatest value that presents in `values` that constitutes a joint majority.
    ///
    /// E.g., for a given membership: [{1,2,3}, {4,5,6}], and a value set: {1:10, 2:20, 5:20, 6:20},
    /// `10` constitutes a majoirty in the first config {1,2,3}.
    /// `20` constitutes a majority in the second config {4,5,6}.
    /// Thus the minimal value `10` is the greatest joint majority for this membership config.
    pub fn greatest_majority_value<'v, V>(&self, values: &'v BTreeMap<NodeId, V>) -> Option<&'v V>
    where V: Ord {
        let mut res = vec![];
        for config in self.configs.iter() {
            let mut vs = Vec::with_capacity(config.len());

            for id in config.iter() {
                let v = values.get(id);
                if let Some(v) = v {
                    vs.push(v)
                }
            }

            let majority = quorum::majority_of(config.len());

            if vs.len() < majority {
                res.push(None);
                continue;
            }

            vs.sort_unstable();

            let majority_greatest = Some(vs[vs.len() - majority]);
            res.push(majority_greatest);
        }

        let min_greatest = res.into_iter().min();
        min_greatest.unwrap_or(None)
    }

    fn is_majority_of_single_config(granted: &BTreeSet<NodeId>, single_config: &BTreeSet<NodeId>) -> bool {
        let d = granted.intersection(single_config);
        let n_granted = d.fold(0, |a, _x| a + 1);

        let majority = quorum::majority_of(single_config.len());
        n_granted >= majority
    }

    fn build_all_nodes(configs: &[BTreeSet<NodeId>]) -> BTreeSet<NodeId> {
        let mut nodes = BTreeSet::new();
        for config in configs.iter() {
            nodes.extend(config)
        }
        nodes
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////

/// An RPC sent by candidates to gather votes (§5.2).
#[derive(Debug, Serialize, Deserialize)]
pub struct VoteRequest {
    /// The candidate's current term.
    pub term: u64,

    pub candidate_id: u64,

    pub last_log_id: LogId,
}

impl MessageSummary for VoteRequest {
    fn summary(&self) -> String {
        format!("{}-{}, last_log:{}", self.term, self.candidate_id, self.last_log_id)
    }
}

impl VoteRequest {
    pub fn new(term: u64, candidate_id: u64, last_log_id: LogId) -> Self {
        Self {
            term,
            candidate_id,
            last_log_id,
        }
    }
}

/// The response to a `VoteRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct VoteResponse {
    /// The current term of the responding node, for the candidate to update itself.
    pub term: u64,

    /// Will be true if the candidate received a vote from the responder.
    pub vote_granted: bool,

    /// The last log id stored on the remote voter.
    pub last_log_id: LogId,
}

//////////////////////////////////////////////////////////////////////////////////////////////////

/// An RPC sent by the Raft leader to send chunks of a snapshot to a follower (§7).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallSnapshotRequest {
    /// The leader's current term.
    pub term: u64,
    /// The leader's ID. Useful in redirecting clients.
    pub leader_id: u64,

    /// Metadata of a snapshot: snapshot_id, last_log_ed membership etc.
    pub meta: SnapshotMeta,

    /// The byte offset where this chunk of data is positioned in the snapshot file.
    pub offset: u64,
    /// The raw bytes of the snapshot chunk, starting at `offset`.
    pub data: Vec<u8>,

    /// Will be `true` if this is the last chunk in the snapshot.
    pub done: bool,
}

impl MessageSummary for InstallSnapshotRequest {
    fn summary(&self) -> String {
        format!(
            "term={}, leader_id={}, meta={:?}, offset={}, len={}, done={}",
            self.term,
            self.leader_id,
            self.meta,
            self.offset,
            self.data.len(),
            self.done
        )
    }
}

/// The response to an `InstallSnapshotRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct InstallSnapshotResponse {
    /// The receiving node's current term, for leader to update itself.
    pub term: u64,
}

//////////////////////////////////////////////////////////////////////////////////////////////////

/// An application specific client request to update the state of the system (§5.1).
///
/// The entry of this payload will be appended to the Raft log and then applied to the Raft state
/// machine according to the Raft protocol.
#[derive(Debug, Serialize, Deserialize)]
pub struct ClientWriteRequest<D: AppData> {
    /// The application specific contents of this client request.
    #[serde(bound = "D: AppData")]
    pub(crate) entry: EntryPayload<D>,
}

impl<D: AppData> MessageSummary for ClientWriteRequest<D> {
    fn summary(&self) -> String {
        self.entry.summary()
    }
}

impl<D: AppData> ClientWriteRequest<D> {
    /// Create a new client payload instance with a normal entry type.
    pub fn new(entry: D) -> Self {
        Self::new_base(EntryPayload::Normal(entry))
    }

    /// Create a new instance.
    pub(crate) fn new_base(entry: EntryPayload<D>) -> Self {
        Self { entry }
    }

    /// Generate a new payload holding a config change.
    pub(crate) fn new_config(membership: Membership) -> Self {
        Self::new_base(EntryPayload::Membership(membership))
    }

    /// Generate a new blank payload.
    ///
    /// This is used by new leaders when first coming to power.
    pub(crate) fn new_blank_payload() -> Self {
        Self::new_base(EntryPayload::Blank)
    }
}

/// The response to a `ClientRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ClientWriteResponse<R: AppDataResponse> {
    pub log_id: LogId,

    /// Application specific response data.
    #[serde(bound = "R: AppDataResponse")]
    pub data: R,

    /// If the log entry is a change-membership entry.
    pub membership: Option<Membership>,
}

impl<R: AppDataResponse> MessageSummary for ClientWriteResponse<R> {
    fn summary(&self) -> String {
        format!("log_id: {}, membership: {:?}", self.log_id, self.membership)
    }
}
