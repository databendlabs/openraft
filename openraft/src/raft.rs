//! Public Raft interface and data types.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tokio::task::JoinError;
use tokio::task::JoinHandle;
use tracing::Level;

use crate::config::Config;
use crate::core::replication_lag;
use crate::core::Expectation;
use crate::core::RaftCore;
use crate::core::Tick;
use crate::error::AddLearnerError;
use crate::error::AppendEntriesError;
use crate::error::CheckIsLeaderError;
use crate::error::ClientWriteError;
use crate::error::Fatal;
use crate::error::InitializeError;
use crate::error::InstallSnapshotError;
use crate::error::VoteError;
use crate::membership::IntoOptionNodes;
use crate::metrics::RaftMetrics;
use crate::metrics::Wait;
use crate::storage::Snapshot;
use crate::AppData;
use crate::AppDataResponse;
use crate::ChangeMembers;
use crate::Entry;
use crate::EntryPayload;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::Node;
use crate::NodeId;
use crate::RaftNetworkFactory;
use crate::RaftState;
use crate::RaftStorage;
use crate::SnapshotMeta;
use crate::Vote;

/// Configuration of types used by the [`Raft`] core engine.
///
/// The (empty) implementation structure defines request/response types, node ID type
/// and the like. Refer to the documentation of associated types for more information.
///
/// ## Note
///
/// Since Rust cannot automatically infer traits for various inner types using this config
/// type as a parameter, this trait simply uses all the traits required for various types
/// as its supertraits as a workaround. To ease the declaration, the macro
/// [`declare_raft_types!`] is provided, which can be used to declare the type easily.
///
/// Example:
/// ```ignore
/// openraft::declare_raft_types!(
///    /// Declare the type configuration for `MemStore`.
///    pub Config: D = ClientRequest, R = ClientResponse, NodeId = MemNodeId
/// );
/// ```
pub trait RaftTypeConfig:
    Sized + Send + Sync + Debug + Clone + Copy + Default + Eq + PartialEq + Ord + PartialOrd + 'static
{
    /// Application-specific request data passed to the state machine.
    type D: AppData;

    /// Application-specific response data returned by the state machine.
    type R: AppDataResponse;

    /// A Raft node's ID.
    type NodeId: NodeId;
}

/// Define types for a Raft type configuration.
///
/// Since Rust has some limitations when deriving traits for types with generic arguments
/// and most types are parameterized by [`RaftTypeConfig`], we need to add supertraits to
/// a type implementing [`RaftTypeConfig`].
///
/// This macro does exactly that.
///
/// Example:
/// ```ignore
/// openraft::declare_raft_types!(
///    /// Declare the type configuration for `MemStore`.
///    pub Config: D = ClientRequest, R = ClientResponse, NodeId = MemNodeId
/// );
/// ```
#[macro_export]
macro_rules! declare_raft_types {
    ( $(#[$outer:meta])* $visibility:vis $id:ident: $($(#[$inner:meta])* $type_id:ident = $type:ty),+ ) => {
        $(#[$outer])*
        #[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
        #[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
        $visibility struct $id {}

        impl $crate::RaftTypeConfig for $id {
            $(
                $(#[$inner])*
                type $type_id = $type;
            )+
        }
    };
}

/// The running state of RaftCore
enum CoreState<NID: NodeId> {
    /// The RaftCore task is still running.
    Running(JoinHandle<Result<(), Fatal<NID>>>),

    /// The RaftCore task has finished. The return value of the task is stored.
    Done(Result<(), Fatal<NID>>),
}

struct RaftInner<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    id: C::NodeId,
    config: Arc<Config>,
    tx_api: mpsc::UnboundedSender<RaftMsg<C, N, S>>,
    rx_metrics: watch::Receiver<RaftMetrics<C::NodeId>>,
    // TODO(xp): it does not need to be a async mutex.
    #[allow(clippy::type_complexity)]
    tx_shutdown: Mutex<Option<oneshot::Sender<()>>>,
    marker_n: std::marker::PhantomData<N>,
    marker_s: std::marker::PhantomData<S>,
    core_state: Mutex<CoreState<C::NodeId>>,
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
pub struct Raft<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    inner: Arc<RaftInner<C, N, S>>,
}

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> Raft<C, N, S> {
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
    /// An implementation of the `RaftNetworkFactory` trait which will be used by Raft for sending RPCs to
    /// peer nodes within the cluster. See the docs on the `RaftNetworkFactory` trait for more details.
    ///
    /// ### `storage`
    /// An implementation of the `RaftStorage` trait which will be used by Raft for data storage.
    /// See the docs on the `RaftStorage` trait for more details.
    #[tracing::instrument(level="debug", skip(config, network, storage), fields(cluster=%config.cluster_name))]
    pub fn new(id: C::NodeId, config: Arc<Config>, network: N, storage: S) -> Self {
        let (tx_api, rx_api) = mpsc::unbounded_channel();
        let (tx_metrics, rx_metrics) = watch::channel(RaftMetrics::new_initial(id));
        let (tx_shutdown, rx_shutdown) = oneshot::channel();

        let _tick_handle = Tick::spawn(Duration::from_millis(config.heartbeat_interval * 3 / 2), tx_api.clone());

        let core_handle = RaftCore::spawn(
            id,
            config.clone(),
            network,
            storage,
            tx_api.clone(),
            rx_api,
            tx_metrics,
            rx_shutdown,
        );

        let inner = RaftInner {
            id,
            config,
            tx_api,
            rx_metrics,
            tx_shutdown: Mutex::new(Some(tx_shutdown)),
            marker_n: std::marker::PhantomData,
            marker_s: std::marker::PhantomData,
            core_state: Mutex::new(CoreState::Running(core_handle)),
        };
        Self { inner: Arc::new(inner) }
    }

    /// Submit an AppendEntries RPC to this Raft node.
    ///
    /// These RPCs are sent by the cluster leader to replicate log entries (§5.3), and are also
    /// used as heartbeats (§5.2).
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub async fn append_entries(
        &self,
        rpc: AppendEntriesRequest<C>,
    ) -> Result<AppendEntriesResponse<C::NodeId>, AppendEntriesError<C::NodeId>> {
        tracing::debug!(rpc = display(rpc.summary()), "Raft::append_entries");

        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::AppendEntries { rpc, tx }, rx).await
    }

    /// Submit a VoteRequest (RequestVote in the spec) RPC to this Raft node.
    ///
    /// These RPCs are sent by cluster peers which are in candidate state attempting to gather votes (§5.2).
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub async fn vote(&self, rpc: VoteRequest<C::NodeId>) -> Result<VoteResponse<C::NodeId>, VoteError<C::NodeId>> {
        tracing::debug!(rpc = display(rpc.summary()), "Raft::vote()");

        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::RequestVote { rpc, tx }, rx).await
    }

    /// Submit an InstallSnapshot RPC to this Raft node.
    ///
    /// These RPCs are sent by the cluster leader in order to bring a new node or a slow node up-to-speed
    /// with the leader (§7).
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub async fn install_snapshot(
        &self,
        rpc: InstallSnapshotRequest<C>,
    ) -> Result<InstallSnapshotResponse<C::NodeId>, InstallSnapshotError<C::NodeId>> {
        tracing::debug!(rpc = display(rpc.summary()), "Raft::install_snapshot()");

        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::InstallSnapshot { rpc, tx }, rx).await
    }

    /// Get the ID of the current leader from this Raft node.
    ///
    /// This method is based on the Raft metrics system which does a good job at staying
    /// up-to-date; however, the `is_leader` method must still be used to guard against stale
    /// reads. This method is perfect for making decisions on where to route client requests.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn current_leader(&self) -> Option<C::NodeId> {
        self.metrics().borrow().current_leader
    }

    /// Check to ensure this node is still the cluster leader, in order to guard against stale reads (§8).
    ///
    /// The actual read operation itself is up to the application, this method just ensures that
    /// the read will not be stale.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn is_leader(&self) -> Result<(), CheckIsLeaderError<C::NodeId>> {
        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::CheckIsLeaderRequest { tx }, rx).await
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
    pub async fn client_write(
        &self,
        rpc: ClientWriteRequest<C>,
    ) -> Result<ClientWriteResponse<C>, ClientWriteError<C::NodeId>> {
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
    /// Once a node successfully initialized it will commit a new membership config
    /// log entry to store.
    /// Then it starts to work, i.e., entering Candidate state and try electing itself as the leader.
    ///
    /// More than one node performing `initialize()` with the same config is safe,
    /// with different config will result in split brain condition.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn initialize<T>(&self, members: T) -> Result<(), InitializeError<C::NodeId>>
    where T: IntoOptionNodes<C::NodeId> + Debug {
        let (tx, rx) = oneshot::channel();
        self.call_core(
            RaftMsg::Initialize {
                members: members.into_option_nodes(),
                tx,
            },
            rx,
        )
        .await
    }

    /// Add a new learner raft node, optionally, blocking until up-to-speed.
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
        id: C::NodeId,
        node: Option<Node>,
        blocking: bool,
    ) -> Result<AddLearnerResponse<C::NodeId>, AddLearnerError<C::NodeId>> {
        let (tx, rx) = oneshot::channel();
        let resp = self.call_core(RaftMsg::AddLearner { id, node, tx }, rx).await?;

        if !blocking {
            return Ok(resp);
        }

        if self.inner.id == id {
            return Ok(resp);
        }

        // Otherwise, blocks until the replication to the new learner becomes up to date.

        // The log id of the membership that contains the added learner.
        let membership_log_id = resp.membership_log_id;

        let res0 = Arc::new(std::sync::Mutex::new(resp));
        let res = res0.clone();

        let wait_res = self
            .wait(None)
            .metrics(
                |metrics| match self.check_replication_upto_date(metrics, id, membership_log_id) {
                    Ok(matched) => {
                        res.lock().unwrap().matched = matched;
                        true
                    }
                    // keep waiting
                    Err(_) => false,
                },
                "wait new learner to become line-rate",
            )
            .await;

        tracing::info!(wait_res = debug(&wait_res), "waiting for replication to new learner");

        let r = {
            let x = res0.lock().unwrap();
            x.clone()
        };
        Ok(r)
    }

    /// Returns Ok() with the latest known matched log id if it should quit waiting: leader change, node removed, or
    /// replication becomes upto date.
    ///
    /// Returns Err() if it should keep waiting.
    fn check_replication_upto_date(
        &self,
        metrics: &RaftMetrics<C::NodeId>,
        node_id: C::NodeId,
        membership_log_id: Option<LogId<C::NodeId>>,
    ) -> Result<Option<LogId<C::NodeId>>, ()> {
        if metrics.membership_config.log_id < membership_log_id {
            // Waiting for the latest metrics to report.
            return Err(());
        }

        if !metrics.membership_config.membership.contains(&node_id) {
            // This learner has been removed.
            return Ok(None);
        }

        let repl = match &metrics.replication {
            None => {
                // This node is no longer a leader.
                return Ok(None);
            }
            Some(x) => x,
        };

        let replication_metrics = &repl.data().replication;
        let target_metrics = match replication_metrics.get(&node_id) {
            None => {
                // Maybe replication is not reported yet. Keep waiting.
                return Err(());
            }
            Some(x) => x,
        };

        let matched = target_metrics.matched();

        let last_log_id = LogId::new(matched.leader_id, metrics.last_log_index.unwrap_or_default());
        let distance = replication_lag(&Some(matched), &Some(last_log_id));

        if distance <= self.inner.config.replication_lag_threshold {
            // replication became up to date.
            return Ok(Some(matched));
        }

        // Not up to date, keep waiting.
        Err(())
    }

    /// Propose a cluster configuration change.
    ///
    /// A node in the proposed config has to be a learner, otherwise it fails with LearnerNotFound error.
    ///
    /// Internally:
    /// - It proposes a **joint** config.
    /// - When the **joint** config is committed, it proposes a uniform config.
    ///
    /// If `allow_lagging` is true, it will always propose the new membership and wait until committed.
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
    /// If it loses leadership or crashed before committing the second **uniform** config log, the cluster is left in
    /// the **joint** config.
    #[tracing::instrument(level = "info", skip_all)]
    pub async fn change_membership(
        &self,
        members: impl Into<ChangeMembers<C::NodeId>>,
        allow_lagging: bool,
        turn_to_learner: bool,
    ) -> Result<ClientWriteResponse<C>, ClientWriteError<C::NodeId>> {
        let changes: ChangeMembers<C::NodeId> = members.into();

        tracing::info!(
            changes = debug(&changes),
            allow_lagging = display(allow_lagging),
            turn_to_learner = display(turn_to_learner),
            "change_membership: start to commit joint config"
        );

        let when = if allow_lagging {
            None
        } else {
            match &changes {
                // Removing voters will never be blocked by replication.
                ChangeMembers::Remove(_) => None,
                _ => Some(Expectation::AtLineRate),
            }
        };

        let (tx, rx) = oneshot::channel();
        // res is error if membership can not be changed.
        // If no error, it will enter a joint state
        let res = self
            .call_core(
                RaftMsg::ChangeMembership {
                    changes: changes.clone(),
                    when: when.clone(),
                    turn_to_learner,
                    tx,
                },
                rx,
            )
            .await?;

        tracing::debug!("res of first step: {:?}", res.summary());

        let (log_id, joint) = (res.log_id, res.membership.clone().unwrap());

        if !joint.is_in_joint_consensus() {
            return Ok(res);
        }

        tracing::debug!("committed a joint config: {} {:?}", log_id, joint);
        tracing::debug!("the second step is to change to uniform config: {:?}", changes);

        let (tx, rx) = oneshot::channel();
        let res = self
            .call_core(
                RaftMsg::ChangeMembership {
                    changes,
                    when,
                    turn_to_learner,
                    tx,
                },
                rx,
            )
            .await?;

        tracing::info!("res of second step of do_change_membership: {}", res.summary());

        Ok(res)
    }

    /// Invoke RaftCore by sending a RaftMsg and blocks waiting for response.
    #[tracing::instrument(level = "debug", skip(self, mes, rx))]
    pub(crate) async fn call_core<T, E>(&self, mes: RaftMsg<C, N, S>, rx: RaftRespRx<T, E>) -> Result<T, E>
    where E: From<Fatal<C::NodeId>> {
        let sum = if tracing::enabled!(Level::DEBUG) {
            None
        } else {
            Some(mes.summary())
        };

        let send_res = self.inner.tx_api.send(mes);

        if send_res.is_err() {
            let fatal = self.get_core_stopped_error("sending tx to RaftCore", sum).await;
            return Err(fatal.into());
        }

        let recv_res = rx.await;

        match recv_res {
            Ok(x) => x,
            Err(_) => {
                let fatal = self.get_core_stopped_error("receiving rx from RaftCore", sum).await;
                Err(fatal.into())
            }
        }
    }

    async fn get_core_stopped_error(&self, when: impl Display, message_summary: Option<String>) -> Fatal<C::NodeId> {
        // Wait for the core task to finish.
        self.join_core_task().await;

        // Retrieve the result.
        let core_res = {
            let state = self.inner.core_state.lock().await;
            if let CoreState::Done(core_task_res) = &*state {
                core_task_res.clone()
            } else {
                unreachable!("RaftCore should have already quit")
            }
        };

        tracing::error!(
            core_result = debug(&core_res),
            "failure {}; message: {:?}",
            when,
            message_summary
        );

        match core_res {
            // A normal quit is still an unexpected "stop" to the caller.
            Ok(_) => Fatal::Stopped,
            Err(e) => e,
        }
    }

    /// Wait for RaftCore task to finish and record the returned value from the task.
    async fn join_core_task(&self) {
        let mut state = self.inner.core_state.lock().await;
        match &mut *state {
            CoreState::Running(handle) => {
                let res = handle.await;
                tracing::info!(res = debug(&res), "RaftCore exited");

                let core_task_res = match res {
                    Err(err) => {
                        if err.is_panic() {
                            Err(Fatal::Panicked)
                        } else {
                            Err(Fatal::Stopped)
                        }
                    }
                    Ok(returned_res) => returned_res,
                };

                *state = CoreState::Done(core_task_res);
            }
            CoreState::Done(_) => {
                // RaftCore has already quit, nothing to do
            }
        }
    }

    /// Send a request to the Raft core loop in a fire-and-forget manner.
    ///
    /// The request functor will be called with a mutable reference to both the state machine
    /// and the network factory and serialized with other Raft core loop processing (e.g., client
    /// requests or general state changes). The current state of the system is passed as well.
    ///
    /// If a response is required, then the caller can store the sender of a one-shot channel
    /// in the closure of the request functor, which can then be used to send the response
    /// asynchronously.
    ///
    /// If the API channel is already closed (Raft is in shutdown), then the request functor is
    /// destroyed right away and not called at all.
    pub fn external_request<F: FnOnce(&RaftState<C::NodeId>, &mut S, &mut N) + Send + 'static>(&self, req: F) {
        let _ignore_error = self.inner.tx_api.send(RaftMsg::ExternalRequest { req: Box::new(req) });
    }

    /// Get a handle to the metrics channel.
    pub fn metrics(&self) -> watch::Receiver<RaftMetrics<C::NodeId>> {
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
    pub fn wait(&self, timeout: Option<Duration>) -> Wait<C::NodeId> {
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
            // A failure to send means the RaftCore is already shutdown. Continue to check the task return value.
            let send_res = tx.send(());
            tracing::info!("sending shutdown signal to RaftCore, sending res: {:?}", send_res);
        }

        self.join_core_task().await;

        // TODO(xp): API change: replace `JoinError` with `Fatal`,
        //           to let the caller know the return value of RaftCore task.
        Ok(())
    }
}

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> Clone for Raft<C, N, S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

pub(crate) type RaftRespTx<T, E> = oneshot::Sender<Result<T, E>>;
pub(crate) type RaftRespRx<T, E> = oneshot::Receiver<Result<T, E>>;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct AddLearnerResponse<NID: NodeId> {
    /// The log id of the membership that contains the added learner.
    pub membership_log_id: Option<LogId<NID>>,

    /// The last log id that matches leader log.
    pub matched: Option<LogId<NID>>,
}

/// A message coming from the Raft API.
pub(crate) enum RaftMsg<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    AppendEntries {
        rpc: AppendEntriesRequest<C>,
        tx: RaftRespTx<AppendEntriesResponse<C::NodeId>, AppendEntriesError<C::NodeId>>,
    },
    RequestVote {
        rpc: VoteRequest<C::NodeId>,
        tx: RaftRespTx<VoteResponse<C::NodeId>, VoteError<C::NodeId>>,
    },
    VoteResponse {
        target: C::NodeId,
        resp: VoteResponse<C::NodeId>,

        /// Which ServerState sent this message. It is also the requested vote.
        vote: Vote<C::NodeId>,
    },
    InstallSnapshot {
        rpc: InstallSnapshotRequest<C>,
        tx: RaftRespTx<InstallSnapshotResponse<C::NodeId>, InstallSnapshotError<C::NodeId>>,
    },

    ClientWriteRequest {
        rpc: ClientWriteRequest<C>,
        tx: RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>,
    },
    CheckIsLeaderRequest {
        tx: RaftRespTx<(), CheckIsLeaderError<C::NodeId>>,
    },

    Initialize {
        members: BTreeMap<C::NodeId, Option<Node>>,
        tx: RaftRespTx<(), InitializeError<C::NodeId>>,
    },
    /// Request raft core to setup a new replication to a learner.
    AddLearner {
        id: C::NodeId,

        node: Option<Node>,

        /// Send the log id when the replication becomes line-rate.
        tx: RaftRespTx<AddLearnerResponse<C::NodeId>, AddLearnerError<C::NodeId>>,
    },
    ChangeMembership {
        changes: ChangeMembers<C::NodeId>,

        /// Defines what conditions the replication states has to satisfy before change membership.
        /// If expectation is not satisfied, a corresponding error will return.
        when: Option<Expectation>,

        /// If `turn_to_learner` is `true`, then all the members which do not exist in the new membership
        /// will be turned into learners, otherwise they will be removed.
        turn_to_learner: bool,

        tx: RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>,
    },

    ExternalRequest {
        #[allow(clippy::type_complexity)]
        req: Box<dyn FnOnce(&RaftState<C::NodeId>, &mut S, &mut N) + Send + 'static>,
    },

    /// A tick event to wake up RaftCore to check timeout etc.
    Tick {
        /// ith tick
        i: usize,
    },

    /// Update the `matched` log id of a replication target.
    /// Sent by a replication task `ReplicationCore`.
    UpdateReplicationMatched {
        /// The ID of the target node for which the match index is to be updated.
        target: C::NodeId,

        /// Either the last log id that has been successfully replicated to the target,
        /// or an error in string.
        result: Result<LogId<C::NodeId>, String>,

        /// Which ServerState sent this message
        vote: Vote<C::NodeId>,
    },

    /// An event indicating that the Raft node needs to revert to follower state.
    /// Sent by a replication task `ReplicationCore`.
    // TODO: rename it
    RevertToFollower {
        /// The ID of the target node from which the new term was observed.
        target: C::NodeId,

        /// The new vote observed.
        new_vote: Vote<C::NodeId>,

        /// Which ServerState sent this message
        vote: Vote<C::NodeId>,
    },

    /// An event from a replication stream requesting snapshot info.
    /// Sent by a replication task `ReplicationCore`.
    NeedsSnapshot {
        target: C::NodeId,

        /// The log id the caller requires the snapshot has to include.
        must_include: Option<LogId<C::NodeId>>,

        /// The response channel for delivering the snapshot data.
        tx: oneshot::Sender<Snapshot<C::NodeId, S::SnapshotData>>,

        /// Which ServerState sent this message
        vote: Vote<C::NodeId>,
    },

    /// Some critical error has taken place, and Raft needs to shutdown.
    /// Sent by a replication task `ReplicationCore`.
    ReplicationFatal,
}

impl<C, N, S> MessageSummary<RaftMsg<C, N, S>> for RaftMsg<C, N, S>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    S: RaftStorage<C>,
{
    fn summary(&self) -> String {
        match self {
            RaftMsg::AppendEntries { rpc, .. } => {
                format!("AppendEntries: {}", rpc.summary())
            }
            RaftMsg::RequestVote { rpc, .. } => {
                format!("RequestVote: {}", rpc.summary())
            }
            RaftMsg::VoteResponse { target, resp, vote } => {
                format!("VoteResponse: from: {}: {}, res-vote: {}", target, resp.summary(), vote)
            }
            RaftMsg::InstallSnapshot { rpc, .. } => {
                format!("InstallSnapshot: {}", rpc.summary())
            }
            RaftMsg::ClientWriteRequest { rpc, .. } => {
                format!("ClientWriteRequest: {}", rpc.summary())
            }
            RaftMsg::CheckIsLeaderRequest { .. } => "CheckIsLeaderRequest".to_string(),
            RaftMsg::Initialize { members, .. } => {
                format!("Initialize: {:?}", members)
            }
            RaftMsg::AddLearner { id, node, .. } => {
                format!("AddLearner: id: {}, node: {:?}", id, node)
            }
            RaftMsg::ChangeMembership {
                changes: members,
                when,
                turn_to_learner,
                ..
            } => {
                format!(
                    "ChangeMembership: members: {:?}, when: {:?}, turn_to_learner: {}",
                    members, when, turn_to_learner,
                )
            }
            RaftMsg::ExternalRequest { .. } => "External Request".to_string(),
            RaftMsg::Tick { i } => {
                format!("Tick {}", i)
            }
            RaftMsg::UpdateReplicationMatched {
                ref target,
                ref result,
                ref vote,
            } => {
                format!(
                    "UpdateMatchIndex: target: {}, result: {:?}, server_state_vote: {}",
                    target, result, vote
                )
            }
            RaftMsg::RevertToFollower {
                ref target,
                ref new_vote,
                ref vote,
            } => {
                format!(
                    "RevertToFollower: target: {}, vote: {}, server_state_vote: {}",
                    target, new_vote, vote
                )
            }
            RaftMsg::NeedsSnapshot {
                ref target,
                ref must_include,
                ref vote,
                ..
            } => {
                format!(
                    "NeedsSnapshot: target: {}, must_include: {:?}, server_state_vote: {}",
                    target, must_include, vote
                )
            }
            RaftMsg::ReplicationFatal => "ReplicationFatal".to_string(),
        }
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////

/// An RPC sent by a cluster leader to replicate log entries (§5.3), and as a heartbeat (§5.2).
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct AppendEntriesRequest<C: RaftTypeConfig> {
    pub vote: Vote<C::NodeId>,

    pub prev_log_id: Option<LogId<C::NodeId>>,

    /// The new log entries to store.
    ///
    /// This may be empty when the leader is sending heartbeats. Entries
    /// are batched for efficiency.
    pub entries: Vec<Entry<C>>,

    /// The leader's committed log id.
    pub leader_commit: Option<LogId<C::NodeId>>,
}

impl<C: RaftTypeConfig> Clone for AppendEntriesRequest<C> {
    fn clone(&self) -> Self {
        Self {
            vote: self.vote,
            prev_log_id: self.prev_log_id,
            entries: self.entries.clone(),
            leader_commit: self.leader_commit,
        }
    }
}

impl<C: RaftTypeConfig> Debug for AppendEntriesRequest<C>
where C::D: Debug
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppendEntriesRequest")
            .field("vote", &self.vote)
            .field("prev_log_id", &self.prev_log_id)
            .field("entries", &self.entries)
            .field("leader_commit", &self.leader_commit)
            .finish()
    }
}

impl<C: RaftTypeConfig> MessageSummary<AppendEntriesRequest<C>> for AppendEntriesRequest<C> {
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
#[derive(Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum AppendEntriesResponse<NID: NodeId> {
    Success,
    Conflict,
    HigherVote(Vote<NID>),
}

impl<NID: NodeId> AppendEntriesResponse<NID> {
    pub fn is_success(&self) -> bool {
        matches!(*self, AppendEntriesResponse::Success)
    }

    pub fn is_conflict(&self) -> bool {
        matches!(*self, AppendEntriesResponse::Conflict)
    }
}

impl<NID: NodeId> MessageSummary<AppendEntriesResponse<NID>> for AppendEntriesResponse<NID> {
    fn summary(&self) -> String {
        match self {
            AppendEntriesResponse::Success => "Success".to_string(),
            AppendEntriesResponse::HigherVote(vote) => format!("Higher vote, {}", vote),
            AppendEntriesResponse::Conflict => "Conflict".to_string(),
        }
    }
}

/// An RPC sent by candidates to gather votes (§5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct VoteRequest<NID: NodeId> {
    pub vote: Vote<NID>,
    pub last_log_id: Option<LogId<NID>>,
}

impl<NID: NodeId> MessageSummary<VoteRequest<NID>> for VoteRequest<NID> {
    fn summary(&self) -> String {
        format!("{}, last_log:{:?}", self.vote, self.last_log_id.map(|x| x.to_string()))
    }
}

impl<NID: NodeId> VoteRequest<NID> {
    pub fn new(vote: Vote<NID>, last_log_id: Option<LogId<NID>>) -> Self {
        Self { vote, last_log_id }
    }
}

/// The response to a `VoteRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct VoteResponse<NID: NodeId> {
    /// vote after a node handling vote-reqest.
    /// Thus `resp.vote >= req.vote` always holds.
    pub vote: Vote<NID>,

    /// Will be true if the candidate received a vote from the responder.
    pub vote_granted: bool,

    /// The last log id stored on the remote voter.
    pub last_log_id: Option<LogId<NID>>,
}

impl<NID: NodeId> MessageSummary<VoteResponse<NID>> for VoteResponse<NID> {
    fn summary(&self) -> String {
        format!(
            "{{granted:{}, {}, last_log:{:?}}}",
            self.vote_granted,
            self.vote,
            self.last_log_id.map(|x| x.to_string())
        )
    }
}

/// An RPC sent by the Raft leader to send chunks of a snapshot to a follower (§7).
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct InstallSnapshotRequest<C: RaftTypeConfig> {
    pub vote: Vote<C::NodeId>,

    /// Metadata of a snapshot: snapshot_id, last_log_ed membership etc.
    pub meta: SnapshotMeta<C::NodeId>,

    /// The byte offset where this chunk of data is positioned in the snapshot file.
    pub offset: u64,
    /// The raw bytes of the snapshot chunk, starting at `offset`.
    pub data: Vec<u8>,

    /// Will be `true` if this is the last chunk in the snapshot.
    pub done: bool,
}

impl<C: RaftTypeConfig> MessageSummary<InstallSnapshotRequest<C>> for InstallSnapshotRequest<C> {
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
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct InstallSnapshotResponse<NID: NodeId> {
    pub vote: Vote<NID>,
}

//////////////////////////////////////////////////////////////////////////////////////////////////

/// An application specific client request to update the state of the system (§5.1).
///
/// The entry of this payload will be appended to the Raft log and then applied to the Raft state
/// machine according to the Raft protocol.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct ClientWriteRequest<C: RaftTypeConfig> {
    /// The application specific contents of this client request.
    pub(crate) payload: EntryPayload<C>,
}

impl<C: RaftTypeConfig> Debug for ClientWriteRequest<C>
where C::D: Debug
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientWriteRequest").field("payload", &self.payload).finish()
    }
}

impl<C: RaftTypeConfig> MessageSummary<ClientWriteRequest<C>> for ClientWriteRequest<C> {
    fn summary(&self) -> String {
        self.payload.summary()
    }
}

impl<C: RaftTypeConfig> ClientWriteRequest<C> {
    pub fn new(entry: EntryPayload<C>) -> Self {
        Self { payload: entry }
    }
}

/// The response to a `ClientRequest`.
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound = "C::R: AppDataResponse")
)]
pub struct ClientWriteResponse<C: RaftTypeConfig> {
    /// The id of the log that is applied.
    pub log_id: LogId<C::NodeId>,

    /// Application specific response data.
    pub data: C::R,

    /// If the log entry is a change-membership entry.
    pub membership: Option<Membership<C::NodeId>>,
}

impl<C: RaftTypeConfig> Debug for ClientWriteResponse<C>
where C::R: Debug
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientWriteResponse")
            .field("log_id", &self.log_id)
            .field("data", &self.data)
            .field("membership", &self.membership)
            .finish()
    }
}

impl<C: RaftTypeConfig> MessageSummary<ClientWriteResponse<C>> for ClientWriteResponse<C> {
    fn summary(&self) -> String {
        format!("log_id: {}, membership: {:?}", self.log_id, self.membership)
    }
}
