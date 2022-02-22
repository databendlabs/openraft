//! Public Raft interface and data types.

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tokio::task::JoinError;
use tokio::task::JoinHandle;
use tracing::Span;

use crate::config::Config;
use crate::core::RaftCore;
use crate::error::AddLearnerError;
use crate::error::AppendEntriesError;
use crate::error::ClientReadError;
use crate::error::ClientWriteError;
use crate::error::Fatal;
use crate::error::InitializeError;
use crate::error::InstallSnapshotError;
use crate::error::VoteError;
use crate::membership::EitherNodesOrIds;
use crate::metrics::RaftMetrics;
use crate::metrics::Wait;
use crate::AppData;
use crate::AppDataResponse;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::Node;
use crate::NodeId;
use crate::RaftNetwork;
use crate::RaftStorage;
use crate::SnapshotMeta;
use crate::Vote;

struct RaftInner<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    tx_api: mpsc::UnboundedSender<(RaftMsg<D, R>, Span)>,
    rx_metrics: watch::Receiver<RaftMetrics>,
    raft_handle: Mutex<Option<JoinHandle<Result<(), Fatal>>>>,
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
    #[tracing::instrument(level = "debug", skip(self, rpc), fields(rpc=%rpc.summary()))]
    pub async fn append_entries(
        &self,
        rpc: AppendEntriesRequest<D>,
    ) -> Result<AppendEntriesResponse, AppendEntriesError> {
        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::AppendEntries { rpc, tx }, rx).await
    }

    /// Submit a VoteRequest (RequestVote in the spec) RPC to this Raft node.
    ///
    /// These RPCs are sent by cluster peers which are in candidate state attempting to gather votes (§5.2).
    #[tracing::instrument(level = "debug", skip(self, rpc), fields(rpc=%rpc.summary()))]
    pub async fn vote(&self, rpc: VoteRequest) -> Result<VoteResponse, VoteError> {
        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::RequestVote { rpc, tx }, rx).await
    }

    /// Submit an InstallSnapshot RPC to this Raft node.
    ///
    /// These RPCs are sent by the cluster leader in order to bring a new node or a slow node up-to-speed
    /// with the leader (§7).
    #[tracing::instrument(level = "debug", skip(self, rpc), fields(snapshot_id=%rpc.meta.last_log_id))]
    pub async fn install_snapshot(
        &self,
        rpc: InstallSnapshotRequest,
    ) -> Result<InstallSnapshotResponse, InstallSnapshotError> {
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
    pub async fn initialize<T>(&self, members: T) -> Result<(), InitializeError>
    where T: Into<EitherNodesOrIds> + Debug {
        let (tx, rx) = oneshot::channel();
        self.call_core(
            RaftMsg::Initialize {
                members: members.into(),
                tx,
            },
            rx,
        )
        .await
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
    ///
    /// The caller can attach additional info `node` to this node id.
    /// A `node` can be used to store the network address of a node. Thus an application does not need another store for
    /// mapping node-id to ip-addr when implementing the RaftNetwork.
    #[tracing::instrument(level = "debug", skip(self, id), fields(target=display(id)))]
    pub async fn add_learner(
        &self,
        id: NodeId,
        node: Option<Node>,
        blocking: bool,
    ) -> Result<AddLearnerResponse, AddLearnerError> {
        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::AddLearner { id, node, blocking, tx }, rx).await
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
    /// If `blocking` is true, it blocks until every learner becomes up to date.
    /// Otherwise it returns error `ChangeMembershipError::LearnerIsLagging` if there is a lagging learner.
    ///
    /// If `turn_to_learner` is true, then all the members which not exists in the new membership,
    /// will be turned into learners, otherwise will be removed.
    ///
    /// Example of `turn_to_learner` usage:
    /// If the original membership is {"members":{1,2,3}, "learners":{}}, and call `change_membership`
    /// with `node_list` {3,4,5}, then:
    ///    - If `turn_to_learner` is true, after commit the new membership is {"members":{3,4,5}, "learners":{1,2}}.
    ///    - Otherwise if `turn_to_learner` is false, then the new membership is {"members":{3,4,5}, "learners":{}}, in
    ///      which the members not exists in the new membership just be removed from the cluster.
    ///
    /// if change_membership from {1,2,3} to {}
    /// If it lost leadership or crashed before committing the second **uniform** config log, the cluster is left in the
    /// **joint** config.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn change_membership(
        &self,
        members: BTreeSet<NodeId>,
        blocking: bool,
        turn_to_learner: bool,
    ) -> Result<ClientWriteResponse<R>, ClientWriteError> {
        tracing::info!("change_membership: start to commit joint config");

        let (tx, rx) = oneshot::channel();
        // res is error if membership can not be changed.
        // If it is not error, it will go into a joint state
        let res = self
            .call_core(
                RaftMsg::ChangeMembership {
                    members: members.clone(),
                    blocking,
                    turn_to_learner,
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
        let res = self
            .call_core(
                RaftMsg::ChangeMembership {
                    members,
                    blocking,
                    turn_to_learner,
                    tx,
                },
                rx,
            )
            .await?;

        tracing::info!("res of second change_membership: {}", res.summary());

        Ok(res)
    }

    /// Invoke RaftCore by sending a RaftMsg and blocks waiting for response.
    #[tracing::instrument(level = "debug", skip(self, mes, rx))]
    pub(crate) async fn call_core<T, E>(&self, mes: RaftMsg<D, R>, rx: RaftRespRx<T, E>) -> Result<T, E>
    where E: From<Fatal> {
        let span = tracing::Span::current();

        let sum = if span.is_disabled() { None } else { Some(mes.summary()) };

        let send_res = self.inner.tx_api.send((mes, span));
        if let Err(send_err) = send_res {
            let last_err = self.inner.rx_metrics.borrow().running_state.clone();
            tracing::error!(%send_err, mes=%sum.unwrap_or_default(), last_error=?last_err, "error send tx to RaftCore");

            let err = match last_err {
                Ok(_) => {
                    // normal shutdown, not caused by any error.
                    Fatal::Stopped
                }
                Err(e) => e,
            };

            return Err(err.into());
        }

        let recv_res = rx.await;
        let res = match recv_res {
            Ok(x) => x,
            Err(e) => {
                let last_err = self.inner.rx_metrics.borrow().running_state.clone();
                tracing::error!(%e, mes=%sum.unwrap_or_default(), last_error=?last_err, "error recv rx from RaftCore");

                let err = match last_err {
                    Ok(_) => {
                        // normal shutdown, not caused by any error.
                        Fatal::Stopped
                    }
                    Err(e) => e,
                };

                Err(err.into())
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
    /// r.wait(Some(timeout)).log(Some(3), "log").await?;
    ///
    /// // wait for ever for raft node's current leader to become 3:
    /// r.wait(None).current_leader(2, "wait for leader").await?;
    ///
    /// // wait for raft state to become a follower
    /// r.wait(None).state(State::Follower, "state").await?;
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
    pub async fn shutdown(&self) -> Result<(), JoinError> {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddLearnerResponse {
    pub matched: Option<LogId>,
}

/// A message coming from the Raft API.
pub(crate) enum RaftMsg<D: AppData, R: AppDataResponse> {
    AppendEntries {
        rpc: AppendEntriesRequest<D>,
        tx: RaftRespTx<AppendEntriesResponse, AppendEntriesError>,
    },
    RequestVote {
        rpc: VoteRequest,
        tx: RaftRespTx<VoteResponse, VoteError>,
    },
    InstallSnapshot {
        rpc: InstallSnapshotRequest,
        tx: RaftRespTx<InstallSnapshotResponse, InstallSnapshotError>,
    },
    ClientWriteRequest {
        rpc: ClientWriteRequest<D>,
        tx: RaftRespTx<ClientWriteResponse<R>, ClientWriteError>,
    },
    ClientReadRequest {
        tx: RaftRespTx<(), ClientReadError>,
    },
    Initialize {
        members: EitherNodesOrIds,
        tx: RaftRespTx<(), InitializeError>,
    },
    // TODO(xp): make tx a field of a struct
    /// Request raft core to setup a new replication to a learner.
    AddLearner {
        id: NodeId,

        node: Option<Node>,

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

        /// If turn_to_learner is true, then all the members which not exists in the new membership,
        /// will be turned into learners, otherwise will be removed.
        turn_to_learner: bool,

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
            RaftMsg::ChangeMembership {
                members,
                blocking,
                turn_to_learner,
                ..
            } => {
                format!(
                    "ChangeMembership: members: {:?}, blocking: {}, turn_to_learner: {}",
                    members, blocking, turn_to_learner,
                )
            }
        }
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////

/// An RPC sent by a cluster leader to replicate log entries (§5.3), and as a heartbeat (§5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendEntriesRequest<D: AppData> {
    pub vote: Vote,

    pub prev_log_id: Option<LogId>,

    /// The new log entries to store.
    ///
    /// This may be empty when the leader is sending heartbeats. Entries
    /// are batched for efficiency.
    #[serde(bound = "D: AppData")]
    pub entries: Vec<Entry<D>>,

    /// The leader's committed log id.
    pub leader_commit: Option<LogId>,
}

impl<D: AppData> MessageSummary for AppendEntriesRequest<D> {
    fn summary(&self) -> String {
        format!(
            "vote={}, prev_log_id={}, leader_commit={}, entries={}",
            self.vote,
            self.prev_log_id.summary(),
            self.leader_commit.summary(),
            self.entries.as_slice().summary()
        )
    }
}

/// The response to an `AppendEntriesRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct AppendEntriesResponse {
    pub vote: Vote,
    pub success: bool,
    pub conflict: bool,
}

impl MessageSummary for AppendEntriesResponse {
    fn summary(&self) -> String {
        format!(
            "vote:{}, success:{:?}, conflict:{:?}",
            self.vote, self.success, self.conflict
        )
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
                format!("membership: {}", c.summary())
            }
        }
    }
}

/// An RPC sent by candidates to gather votes (§5.2).
#[derive(Debug, Serialize, Deserialize)]
pub struct VoteRequest {
    pub vote: Vote,
    pub last_log_id: Option<LogId>,
}

impl MessageSummary for VoteRequest {
    fn summary(&self) -> String {
        format!("{}, last_log:{:?}", self.vote, self.last_log_id)
    }
}

impl VoteRequest {
    pub fn new(vote: Vote, last_log_id: Option<LogId>) -> Self {
        Self { vote, last_log_id }
    }
}

/// The response to a `VoteRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct VoteResponse {
    pub vote: Vote,

    /// Will be true if the candidate received a vote from the responder.
    pub vote_granted: bool,

    /// The last log id stored on the remote voter.
    pub last_log_id: Option<LogId>,
}

//////////////////////////////////////////////////////////////////////////////////////////////////

/// An RPC sent by the Raft leader to send chunks of a snapshot to a follower (§7).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallSnapshotRequest {
    pub vote: Vote,

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
            "vote={}, meta={:?}, offset={}, len={}, done={}",
            self.vote,
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
    pub vote: Vote,
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
    pub(crate) payload: EntryPayload<D>,
}

impl<D: AppData> MessageSummary for ClientWriteRequest<D> {
    fn summary(&self) -> String {
        self.payload.summary()
    }
}

impl<D: AppData> ClientWriteRequest<D> {
    pub fn new(entry: EntryPayload<D>) -> Self {
        Self { payload: entry }
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
