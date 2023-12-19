//! Public Raft interface and data types.

mod external_request;
mod message;
mod raft_inner;
mod runtime_config_handle;
mod trigger;

use std::collections::BTreeMap;

pub(crate) use self::external_request::BoxCoreFn;

pub(in crate::raft) mod core_state;

use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use core_state::CoreState;
use maplit::btreemap;
pub use message::AppendEntriesRequest;
pub use message::AppendEntriesResponse;
pub use message::ClientWriteResponse;
pub use message::InstallSnapshotRequest;
pub use message::InstallSnapshotResponse;
pub use message::VoteRequest;
pub use message::VoteResponse;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tracing::trace_span;
use tracing::Instrument;
use tracing::Level;

use crate::config::Config;
use crate::config::RuntimeConfig;
use crate::core::command_state::CommandState;
use crate::core::raft_msg::RaftMsg;
use crate::core::replication_lag;
use crate::core::sm;
use crate::core::RaftCore;
use crate::core::Tick;
use crate::engine::Engine;
use crate::engine::EngineConfig;
use crate::error::CheckIsLeaderError;
use crate::error::ClientWriteError;
use crate::error::Fatal;
use crate::error::InitializeError;
use crate::error::InstallSnapshotError;
use crate::error::RaftError;
use crate::membership::IntoNodes;
use crate::metrics::RaftMetrics;
use crate::metrics::Wait;
use crate::metrics::WaitError;
use crate::network::RaftNetworkFactory;
use crate::raft::raft_inner::RaftInner;
use crate::raft::runtime_config_handle::RuntimeConfigHandle;
use crate::raft::trigger::Trigger;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::AsyncRuntime;
use crate::ChangeMembers;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::MessageSummary;
use crate::RaftState;
pub use crate::RaftTypeConfig;
use crate::StorageHelper;

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
///    pub Config:
///        D            = ClientRequest,
///        R            = ClientResponse,
///        NodeId       = u64,
///        Node         = openraft::BasicNode,
///        Entry        = openraft::Entry<TypeConfig>,
///        SnapshotData = Cursor<Vec<u8>>,
///        AsyncRuntime = openraft::TokioRuntime,
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
/// is shutting down (potentially for data safety reasons due to a storage error), and the
/// `shutdown` method should be called on this type to await the shutdown of the node. If the parent
/// application needs to shutdown the Raft node for any reason, calling `shutdown` will do the
/// trick.
#[derive(Clone)]
pub struct Raft<C>
where C: RaftTypeConfig
{
    inner: Arc<RaftInner<C>>,
}

impl<C> Raft<C>
where C: RaftTypeConfig
{
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
    /// An implementation of the `RaftNetworkFactory` trait which will be used by Raft for sending
    /// RPCs to peer nodes within the cluster. See the docs on the `RaftNetworkFactory` trait
    /// for more details.
    ///
    /// ### `storage`
    /// An implementation of the `RaftStorage` trait which will be used by Raft for data storage.
    /// See the docs on the `RaftStorage` trait for more details.
    #[tracing::instrument(level="debug", skip_all, fields(cluster=%config.cluster_name))]
    pub async fn new<LS, N, SM>(
        id: C::NodeId,
        config: Arc<Config>,
        network: N,
        mut log_store: LS,
        mut state_machine: SM,
    ) -> Result<Self, Fatal<C::NodeId>>
    where
        N: RaftNetworkFactory<C>,
        LS: RaftLogStorage<C>,
        SM: RaftStateMachine<C>,
    {
        let (tx_api, rx_api) = mpsc::unbounded_channel();
        let (tx_notify, rx_notify) = mpsc::unbounded_channel();
        let (tx_metrics, rx_metrics) = watch::channel(RaftMetrics::new_initial(id));
        let (tx_shutdown, rx_shutdown) = oneshot::channel();

        let tick_handle = Tick::spawn(
            Duration::from_millis(config.heartbeat_interval * 3 / 2),
            tx_notify.clone(),
            config.enable_tick,
        );

        let runtime_config = Arc::new(RuntimeConfig::new(&config));

        let core_span = tracing::span!(
            parent: tracing::Span::current(),
            Level::DEBUG,
            "RaftCore",
            id = display(id),
            cluster = display(&config.cluster_name)
        );

        let eng_config = EngineConfig::new(id, config.as_ref());

        let state = {
            let mut helper = StorageHelper::new(&mut log_store, &mut state_machine);
            helper.get_initial_state().await?
        };

        let engine = Engine::new(state, eng_config);

        let sm_handle = sm::Worker::spawn(state_machine, tx_notify.clone());

        let core: RaftCore<C, N, LS, SM> = RaftCore {
            id,
            config: config.clone(),
            runtime_config: runtime_config.clone(),
            network,
            log_store,
            sm_handle,

            engine,

            client_resp_channels: BTreeMap::new(),

            leader_data: None,

            tx_api: tx_api.clone(),
            rx_api,

            tx_notify,
            rx_notify,

            tx_metrics,

            command_state: CommandState::default(),
            span: core_span,

            _p: Default::default(),
        };

        let core_handle = C::AsyncRuntime::spawn(core.main(rx_shutdown).instrument(trace_span!("spawn").or_current()));

        let inner = RaftInner {
            id,
            config,
            runtime_config,
            tick_handle,
            tx_api,
            rx_metrics,
            tx_shutdown: Mutex::new(Some(tx_shutdown)),
            core_state: Mutex::new(CoreState::Running(core_handle)),
        };

        Ok(Self { inner: Arc::new(inner) })
    }

    /// Return a handle to update runtime config.
    ///
    /// Such enabling/disabling heartbeat, election, etc.
    ///
    /// Example:
    /// ```ignore
    /// let raft = Raft::new(...).await?;
    /// raft.runtime_config().heartbeat(true);
    /// ```
    pub fn runtime_config(&self) -> RuntimeConfigHandle<C> {
        RuntimeConfigHandle::new(self.inner.as_ref())
    }

    /// Enable or disable raft internal ticker.
    #[deprecated(note = "use `Raft::runtime_config().tick()` instead")]
    pub fn enable_tick(&self, enabled: bool) {
        self.runtime_config().tick(enabled)
    }

    #[deprecated(note = "use `Raft::runtime_config().heartbeat()` instead")]
    pub fn enable_heartbeat(&self, enabled: bool) {
        self.runtime_config().heartbeat(enabled)
    }

    #[deprecated(note = "use `Raft::runtime_config().elect()` instead")]
    pub fn enable_elect(&self, enabled: bool) {
        self.runtime_config().elect(enabled)
    }

    /// Return a handle to manually trigger raft actions, such as elect or build snapshot.
    ///
    /// Example:
    /// ```ignore
    /// let raft = Raft::new(...).await?;
    /// raft.trigger().elect().await?;
    /// ```
    pub fn trigger(&self) -> Trigger<C> {
        Trigger::new(self.inner.as_ref())
    }

    /// Trigger election at once and return at once.
    #[deprecated(note = "use `Raft::trigger().elect()` instead")]
    pub async fn trigger_elect(&self) -> Result<(), Fatal<C::NodeId>> {
        self.trigger().elect().await
    }

    /// Trigger a heartbeat at once and return at once.
    #[deprecated(note = "use `Raft::trigger().heartbeat()` instead")]
    pub async fn trigger_heartbeat(&self) -> Result<(), Fatal<C::NodeId>> {
        self.trigger().heartbeat().await
    }

    /// Trigger to build a snapshot at once and return at once.
    #[deprecated(note = "use `Raft::trigger().snapshot()` instead")]
    pub async fn trigger_snapshot(&self) -> Result<(), Fatal<C::NodeId>> {
        self.trigger().snapshot().await
    }

    /// Initiate the log purge up to and including the given `upto` log index.
    #[deprecated(note = "use `Raft::trigger().purge_log()` instead")]
    pub async fn purge_log(&self, upto: u64) -> Result<(), Fatal<C::NodeId>> {
        self.trigger().purge_log(upto).await
    }

    /// Submit an AppendEntries RPC to this Raft node.
    ///
    /// These RPCs are sent by the cluster leader to replicate log entries (§5.3), and are also
    /// used as heartbeats (§5.2).
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub async fn append_entries(
        &self,
        rpc: AppendEntriesRequest<C>,
    ) -> Result<AppendEntriesResponse<C::NodeId>, RaftError<C::NodeId>> {
        tracing::debug!(rpc = display(rpc.summary()), "Raft::append_entries");

        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::AppendEntries { rpc, tx }, rx).await
    }

    /// Submit a VoteRequest (RequestVote in the spec) RPC to this Raft node.
    ///
    /// These RPCs are sent by cluster peers which are in candidate state attempting to gather votes
    /// (§5.2).
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub async fn vote(&self, rpc: VoteRequest<C::NodeId>) -> Result<VoteResponse<C::NodeId>, RaftError<C::NodeId>> {
        tracing::info!(rpc = display(rpc.summary()), "Raft::vote()");

        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::RequestVote { rpc, tx }, rx).await
    }

    /// Submit an InstallSnapshot RPC to this Raft node.
    ///
    /// These RPCs are sent by the cluster leader in order to bring a new node or a slow node
    /// up-to-speed with the leader (§7).
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub async fn install_snapshot(
        &self,
        rpc: InstallSnapshotRequest<C>,
    ) -> Result<InstallSnapshotResponse<C::NodeId>, RaftError<C::NodeId, InstallSnapshotError>> {
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

    /// Check to ensure this node is still the cluster leader, in order to guard against stale reads
    /// (§8).
    ///
    /// The actual read operation itself is up to the application, this method just ensures that
    /// the read will not be stale.
    #[deprecated(note = "use `Raft::ensure_linearizable()` instead. deprecated since 0.9.0")]
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn is_leader(&self) -> Result<(), RaftError<C::NodeId, CheckIsLeaderError<C::NodeId, C::Node>>> {
        let (tx, rx) = oneshot::channel();
        let _ = self.call_core(RaftMsg::CheckIsLeaderRequest { tx }, rx).await?;
        Ok(())
    }

    /// Ensures a read operation performed following this method are linearizable across the
    /// cluster.
    ///
    /// This method is just a shorthand for calling [`get_read_log_id()`](Raft::get_read_log_id) and
    /// then calling [Raft::wait].
    ///
    /// This method confirms the node's leadership at the time of invocation by sending
    /// heartbeats to a quorum of followers, and the state machine is up to date.
    /// This method blocks until all these conditions are met.
    ///
    /// Returns:
    /// - `Ok(read_log_id)` on successful confirmation that the node is the leader. `read_log_id`
    ///   represents the log id up to which the state machine has applied to ensure a linearizable
    ///   read.
    /// - `Err(RaftError<CheckIsLeaderError>)` if it detects a higher term, or if it fails to
    ///   communicate with a quorum of followers.
    ///
    /// # Examples
    /// ```ignore
    /// my_raft.ensure_linearizable().await?;
    /// // Proceed with the state machine read
    /// ```
    /// Read more about how it works: [Read Operation](crate::docs::protocol::read)
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn ensure_linearizable(
        &self,
    ) -> Result<Option<LogId<C::NodeId>>, RaftError<C::NodeId, CheckIsLeaderError<C::NodeId, C::Node>>> {
        let (read_log_id, applied) = self.get_read_log_id().await?;

        if read_log_id.index() > applied.index() {
            self.wait(None)
                .applied_index_at_least(read_log_id.index(), "ensure_linearizable")
                .await
                .map_err(|e| match e {
                    WaitError::Timeout(_, _) => {
                        unreachable!("did not specify timeout")
                    }
                    WaitError::ShuttingDown => Fatal::Stopped,
                })?;
        }
        Ok(read_log_id)
    }

    /// Ensures this node is leader and returns the log id up to which the state machine should
    /// apply to ensure a read can be linearizable across the cluster.
    ///
    /// The leadership is ensured by sending heartbeats to a quorum of followers.
    /// Note that this is just the first step for linearizable read. The second step is to wait for
    /// state machine to reach the returned `read_log_id`.
    ///
    /// Returns:
    /// - `Ok((read_log_id, last_applied_log_id))` on successful confirmation that the node is the
    ///   leader. `read_log_id` represents the log id up to which the state machine should apply to
    ///   ensure a linearizable read.
    /// - `Err(RaftError<CheckIsLeaderError>)` if it detects a higher term, or if it fails to
    ///   communicate with a quorum of followers.
    ///
    /// The caller should then wait for `last_applied_log_id` to catch up, which can be done by
    /// subscribing to [`Raft::metrics`] and waiting for `last_applied_log_id` to
    /// reach `read_log_id`.
    ///
    /// # Examples
    /// ```ignore
    /// let (read_log_id, applied_log_id) = my_raft.get_read_log_id().await?;
    /// if read_log_id.index() > applied_log_id.index() {
    ///     my_raft.wait(None).applied_index_at_least(read_log_id.index()).await?;
    /// }
    /// // Proceed with the state machine read
    /// ```
    /// The comparison `read_log_id > applied_log_id` would also be valid in the above example.
    ///
    /// See: [Read Operation](crate::docs::protocol::read)
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_read_log_id(
        &self,
    ) -> Result<
        (Option<LogId<C::NodeId>>, Option<LogId<C::NodeId>>),
        RaftError<C::NodeId, CheckIsLeaderError<C::NodeId, C::Node>>,
    > {
        let (tx, rx) = oneshot::channel();
        let (read_log_id, applied) = self.call_core(RaftMsg::CheckIsLeaderRequest { tx }, rx).await?;
        Ok((read_log_id, applied))
    }

    /// Submit a mutating client request to Raft to update the state of the system (§5.1).
    ///
    /// It will be appended to the log, committed to the cluster, and then applied to the
    /// application state machine. The result of applying the request to the state machine will
    /// be returned as the response from this method.
    ///
    /// Our goal for Raft is to implement linearizable semantics. If the leader crashes after
    /// committing a log entry but before responding to the client, the client may retry the
    /// command with a new leader, causing it to be executed a second time. As such, clients
    /// should assign unique serial numbers to every command. Then, the state machine should
    /// track the latest serial number processed for each client, along with the associated
    /// response. If it receives a command whose serial number has already been executed, it
    /// responds immediately without re-executing the request (§8). The
    /// `RaftStorage::apply_entry_to_state_machine` method is the perfect place to implement
    /// this.
    ///
    /// These are application specific requirements, and must be implemented by the application
    /// which is being built on top of Raft.
    #[tracing::instrument(level = "debug", skip(self, app_data))]
    pub async fn client_write(
        &self,
        app_data: C::D,
    ) -> Result<ClientWriteResponse<C>, RaftError<C::NodeId, ClientWriteError<C::NodeId, C::Node>>> {
        let (tx, rx) = oneshot::channel();
        self.call_core(RaftMsg::ClientWriteRequest { app_data, tx }, rx).await
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
    /// Then it starts to work, i.e., entering Candidate state and try electing itself as the
    /// leader.
    ///
    /// More than one node performing `initialize()` with the same config is safe,
    /// with different config will result in split brain condition.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn initialize<T>(
        &self,
        members: T,
    ) -> Result<(), RaftError<C::NodeId, InitializeError<C::NodeId, C::Node>>>
    where
        T: IntoNodes<C::NodeId, C::Node> + Debug,
    {
        let (tx, rx) = oneshot::channel();
        self.call_core(
            RaftMsg::Initialize {
                members: members.into_nodes(),
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
    /// If `blocking` is `true`, this function blocks until the leader believes the logs on the new
    /// node is up to date, i.e., ready to join the cluster, as a voter, by calling
    /// `change_membership`.
    ///
    /// If blocking is `false`, this function returns at once as successfully setting up the
    /// replication.
    ///
    /// If the node to add is already a voter or learner, it will still re-add it.
    ///
    /// A `node` is able to store the network address of a node. Thus an application does not
    /// need another store for mapping node-id to ip-addr when implementing the RaftNetwork.
    #[tracing::instrument(level = "debug", skip(self, id), fields(target=display(id)))]
    pub async fn add_learner(
        &self,
        id: C::NodeId,
        node: C::Node,
        blocking: bool,
    ) -> Result<ClientWriteResponse<C>, RaftError<C::NodeId, ClientWriteError<C::NodeId, C::Node>>> {
        let (tx, rx) = oneshot::channel();
        let resp = self
            .call_core(
                RaftMsg::ChangeMembership {
                    changes: ChangeMembers::AddNodes(btreemap! {id=>node}),
                    retain: true,
                    tx,
                },
                rx,
            )
            .await?;

        if !blocking {
            return Ok(resp);
        }

        if self.inner.id == id {
            return Ok(resp);
        }

        // Otherwise, blocks until the replication to the new learner becomes up to date.

        // The log id of the membership that contains the added learner.
        let membership_log_id = resp.log_id;

        let wait_res = self
            .wait(None)
            .metrics(
                |metrics| match self.check_replication_upto_date(metrics, id, Some(membership_log_id)) {
                    Ok(_matching) => true,
                    // keep waiting
                    Err(_) => false,
                },
                "wait new learner to become line-rate",
            )
            .await;

        tracing::info!(wait_res = debug(&wait_res), "waiting for replication to new learner");

        Ok(resp)
    }

    /// Returns Ok() with the latest known matched log id if it should quit waiting: leader change,
    /// node removed, or replication becomes upto date.
    ///
    /// Returns Err() if it should keep waiting.
    fn check_replication_upto_date(
        &self,
        metrics: &RaftMetrics<C::NodeId, C::Node>,
        node_id: C::NodeId,
        membership_log_id: Option<LogId<C::NodeId>>,
    ) -> Result<Option<LogId<C::NodeId>>, ()> {
        if metrics.membership_config.log_id() < &membership_log_id {
            // Waiting for the latest metrics to report.
            return Err(());
        }

        if metrics.membership_config.membership().get_node(&node_id).is_none() {
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

        let replication_metrics = repl;
        let target_metrics = match replication_metrics.get(&node_id) {
            None => {
                // Maybe replication is not reported yet. Keep waiting.
                return Err(());
            }
            Some(x) => x,
        };

        let matched = *target_metrics;

        let distance = replication_lag(&matched.index(), &metrics.last_log_index);

        if distance <= self.inner.config.replication_lag_threshold {
            // replication became up to date.
            return Ok(matched);
        }

        // Not up to date, keep waiting.
        Err(())
    }

    /// Propose a cluster configuration change.
    ///
    /// A node in the proposed config has to be a learner, otherwise it fails with LearnerNotFound
    /// error.
    ///
    /// Internally:
    /// - It proposes a **joint** config.
    /// - When the **joint** config is committed, it proposes a uniform config.
    ///
    /// If `retain` is true, then all the members which not exists in the new membership,
    /// will be turned into learners, otherwise will be removed.
    ///
    /// Example of `retain` usage:
    /// If the original membership is {"voter":{1,2,3}, "learners":{}}, and call
    /// `change_membership` with `voters` {3,4,5}, then:
    ///    - If `retain` is `true`, the committed new membership is {"voters":{3,4,5},
    ///      "learners":{1,2}}.
    ///    - Otherwise if `retain` is `false`, then the new membership is {"voters":{3,4,5},
    ///      "learners":{}}, in which the voters not exists in the new membership just be removed
    ///      from the cluster.
    ///
    /// If it loses leadership or crashed before committing the second **uniform** config log, the
    /// cluster is left in the **joint** config.
    #[tracing::instrument(level = "info", skip_all)]
    pub async fn change_membership(
        &self,
        members: impl Into<ChangeMembers<C::NodeId, C::Node>>,
        retain: bool,
    ) -> Result<ClientWriteResponse<C>, RaftError<C::NodeId, ClientWriteError<C::NodeId, C::Node>>> {
        let changes: ChangeMembers<C::NodeId, C::Node> = members.into();

        tracing::info!(
            changes = debug(&changes),
            retain = display(retain),
            "change_membership: start to commit joint config"
        );

        let (tx, rx) = oneshot::channel();
        // res is error if membership can not be changed.
        // If no error, it will enter a joint state
        let res = self
            .call_core(
                RaftMsg::ChangeMembership {
                    changes: changes.clone(),
                    retain,
                    tx,
                },
                rx,
            )
            .await;

        if let Err(e) = &res {
            tracing::error!("the first step error: {}", e);
        }
        let res = res?;

        tracing::debug!("res of first step: {:?}", res.summary());

        let (log_id, joint) = (res.log_id, res.membership.clone().unwrap());

        if joint.get_joint_config().len() == 1 {
            return Ok(res);
        }

        tracing::debug!("committed a joint config: {} {:?}", log_id, joint);
        tracing::debug!("the second step is to change to uniform config: {:?}", changes);

        let (tx, rx) = oneshot::channel();
        let res = self.call_core(RaftMsg::ChangeMembership { changes, retain, tx }, rx).await;

        if let Err(e) = &res {
            tracing::error!("the second step error: {}", e);
        }
        let res = res?;

        tracing::info!("res of second step of do_change_membership: {}", res.summary());

        Ok(res)
    }

    /// Invoke RaftCore by sending a RaftMsg and blocks waiting for response.
    #[tracing::instrument(level = "debug", skip(self, mes, rx))]
    pub(crate) async fn call_core<T, E>(
        &self,
        mes: RaftMsg<C>,
        rx: oneshot::Receiver<Result<T, E>>,
    ) -> Result<T, RaftError<C::NodeId, E>>
    where
        E: Debug,
    {
        let sum = if tracing::enabled!(Level::DEBUG) {
            Some(mes.summary())
        } else {
            None
        };

        let send_res = self.inner.tx_api.send(mes);

        if send_res.is_err() {
            let fatal = self.inner.get_core_stopped_error("sending tx to RaftCore", sum).await;
            return Err(RaftError::Fatal(fatal));
        }

        let recv_res = rx.await;
        tracing::debug!("call_core receives result is error: {:?}", recv_res.is_err());

        match recv_res {
            Ok(x) => x.map_err(|e| RaftError::APIError(e)),
            Err(_) => {
                let fatal = self.inner.get_core_stopped_error("receiving rx from RaftCore", sum).await;
                tracing::error!(error = debug(&fatal), "core_call fatal error");
                Err(RaftError::Fatal(fatal))
            }
        }
    }

    /// Provides read-only access to [`RaftState`] through a user-provided function.
    ///
    /// The function `func` is applied to the current [`RaftState`]. The result of this function,
    /// of type `V`, is returned wrapped in `Result<V, Fatal<C::NodeId>>`. `Fatal` error will be
    /// returned if failed to receive a reply from `RaftCore`.
    ///
    /// A `Fatal` error is returned if:
    /// - Raft core task is stopped normally.
    /// - Raft core task is panicked due to programming error.
    /// - Raft core task is encountered a storage error.
    ///
    /// Example for getting the current committed log id:
    /// ```ignore
    /// let committed = my_raft.with_raft_state(|st| st.committed).await?;
    /// ```
    pub async fn with_raft_state<F, V>(&self, func: F) -> Result<V, Fatal<C::NodeId>>
    where
        F: FnOnce(&RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>) -> V + Send + 'static,
        V: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();

        self.external_request(|st| {
            let result = func(st);
            if let Err(_err) = tx.send(result) {
                tracing::error!("{}: to-Raft tx send error", func_name!());
            }
        });

        match rx.await {
            Ok(res) => Ok(res),
            Err(err) => {
                tracing::error!(error = display(&err), "{}: rx recv error", func_name!());

                let when = format!("{}: rx recv", func_name!());
                let fatal = self.inner.get_core_stopped_error(when, None::<u64>).await;
                Err(fatal)
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
    pub fn external_request<F>(&self, req: F)
    where F: FnOnce(&RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>) + Send + 'static {
        let req: BoxCoreFn<C> = Box::new(req);
        let _ignore_error = self.inner.tx_api.send(RaftMsg::ExternalRequest { req });
    }

    /// Get a handle to the metrics channel.
    pub fn metrics(&self) -> watch::Receiver<RaftMetrics<C::NodeId, C::Node>> {
        self.inner.rx_metrics.clone()
    }

    /// Get a handle to wait for the metrics to satisfy some condition.
    ///
    /// If `timeout` is `None`, then it will wait forever(10 years).
    /// If `timeout` is `Some`, then it will wait for the specified duration.
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
    pub fn wait(&self, timeout: Option<Duration>) -> Wait<C::NodeId, C::Node, C::AsyncRuntime> {
        let timeout = match timeout {
            Some(t) => t,
            None => Duration::from_secs(86400 * 365 * 100),
        };
        Wait {
            timeout,
            rx: self.inner.rx_metrics.clone(),
            _phantom: PhantomData,
        }
    }

    /// Shutdown this Raft node.
    ///
    /// It sends a shutdown signal and waits until `RaftCore` returns.
    pub async fn shutdown(&self) -> Result<(), <C::AsyncRuntime as AsyncRuntime>::JoinError> {
        if let Some(tx) = self.inner.tx_shutdown.lock().await.take() {
            // A failure to send means the RaftCore is already shutdown. Continue to check the task
            // return value.
            let send_res = tx.send(());
            tracing::info!("sending shutdown signal to RaftCore, sending res: {:?}", send_res);
        }
        self.inner.join_core_task().await;
        self.inner.tick_handle.shutdown().await;

        // TODO(xp): API change: replace `JoinError` with `Fatal`,
        //           to let the caller know the return value of RaftCore task.
        Ok(())
    }
}
