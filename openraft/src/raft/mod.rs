//! Public Raft interface and data types.
//!
//! [`Raft`] serves as the primary interface to a Raft node,
//! facilitating all interactions with the underlying RaftCore.
//!
//! While `RaftCore` operates as a singleton within an application, [`Raft`] instances are designed
//! to be cheaply cloneable.
//! This allows multiple components within the application that require interaction with `RaftCore`
//! to efficiently share access.

#[cfg(test)] mod declare_raft_types_test;
mod external_request;
mod impl_raft_blocking_write;
pub(crate) mod message;
mod raft_inner;
pub mod responder;
mod runtime_config_handle;
pub mod trigger;

use std::collections::BTreeMap;
use std::error::Error;

pub(crate) use self::external_request::BoxCoreFn;

pub(in crate::raft) mod core_state;

use std::fmt::Debug;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use core_state::CoreState;
pub use message::AppendEntriesRequest;
pub use message::AppendEntriesResponse;
pub use message::ClientWriteResponse;
pub use message::ClientWriteResult;
pub use message::InstallSnapshotRequest;
pub use message::InstallSnapshotResponse;
pub use message::SnapshotResponse;
pub use message::VoteRequest;
pub use message::VoteResponse;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tracing::trace_span;
use tracing::Instrument;
use tracing::Level;

use crate::async_runtime::AsyncOneshotSendExt;
use crate::config::Config;
use crate::config::RuntimeConfig;
use crate::core::command_state::CommandState;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::core::raft_msg::RaftMsg;
use crate::core::replication_lag;
use crate::core::sm::worker;
use crate::core::RaftCore;
use crate::core::Tick;
use crate::engine::Engine;
use crate::engine::EngineConfig;
use crate::error::CheckIsLeaderError;
use crate::error::ClientWriteError;
use crate::error::Fatal;
use crate::error::Infallible;
use crate::error::InitializeError;
use crate::error::RaftError;
use crate::membership::IntoNodes;
use crate::metrics::RaftDataMetrics;
use crate::metrics::RaftMetrics;
use crate::metrics::RaftServerMetrics;
use crate::metrics::Wait;
use crate::metrics::WaitError;
use crate::raft::raft_inner::RaftInner;
use crate::raft::responder::Responder;
pub use crate::raft::runtime_config_handle::RuntimeConfigHandle;
use crate::raft::trigger::Trigger;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::storage::LogEventChannel;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::type_config::alias::JoinErrorOf;
use crate::type_config::alias::ResponderOf;
use crate::type_config::alias::ResponderReceiverOf;
use crate::type_config::alias::SnapshotDataOf;
use crate::AsyncRuntime;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::OptionalSend;
use crate::RaftNetworkFactory;
use crate::RaftState;
pub use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::StorageHelper;
use crate::Vote;

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
///    pub TypeConfig:
///        D            = ClientRequest,
///        R            = ClientResponse,
///        NodeId       = u64,
///        Node         = openraft::BasicNode,
///        Entry        = openraft::Entry<TypeConfig>,
///        SnapshotData = Cursor<Vec<u8>>,
///        Responder    = openraft::impls::OneshotResponder<TypeConfig>,
///        AsyncRuntime = openraft::TokioRuntime,
/// );
/// ```
///
/// Types can be omitted, and the following default type will be used:
/// - `D`:            `String`
/// - `R`:            `String`
/// - `NodeId`:       `u64`
/// - `Node`:         `::openraft::impls::BasicNode`
/// - `Entry`:        `::openraft::impls::Entry<Self>`
/// - `SnapshotData`: `Cursor<Vec<u8>>`
/// - `Responder`:    `::openraft::impls::OneshotResponder<Self>`
/// - `AsyncRuntime`: `::openraft::impls::TokioRuntime`
///
/// For example, to declare with only `D` and `R` types:
/// ```ignore
/// openraft::declare_raft_types!(
///    pub TypeConfig:
///        D = ClientRequest,
///        R = ClientResponse,
/// );
/// ```
///
/// Or just use the default type config:
/// ```ignore
/// openraft::declare_raft_types!(pub TypeConfig);
/// ```
#[macro_export]
macro_rules! declare_raft_types {
    // Add a trailing colon to    `declare_raft_types(MyType)`,
    // Make it the standard form: `declare_raft_types(MyType:)`.
    ($(#[$outer:meta])* $visibility:vis $id:ident) => {
        $crate::declare_raft_types!($(#[$outer])* $visibility $id:);
    };

    // The main entry of this macro
    ($(#[$outer:meta])* $visibility:vis $id:ident: $($(#[$inner:meta])* $type_id:ident = $type:ty),* $(,)? ) => {
        $(#[$outer])*
        #[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
        #[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
        $visibility struct $id {}

        impl $crate::RaftTypeConfig for $id {
            // `expand!(KEYED, ...)` ignores the duplicates.
            // Thus by appending default types after user defined types,
            // the absent user defined types are filled with default types.
            $crate::openraft_macros::expand!(
                KEYED,
                (T, ATTR, V) => {ATTR type T = V;},
                $(($type_id, $(#[$inner])*, $type),)*

                // Default types:
                (D            , , String                                ),
                (R            , , String                                ),
                (NodeId       , , u64                                   ),
                (Node         , , $crate::impls::BasicNode              ),
                (Entry        , , $crate::impls::Entry<Self>            ),
                (SnapshotData , , Cursor<Vec<u8>>                       ),
                (Responder    , , $crate::impls::OneshotResponder<Self> ),
                (AsyncRuntime , , $crate::impls::TokioRuntime           ),
            );

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
/// ### Clone
///
/// This type implements `Clone`, and cloning itself is very cheap and helps to facilitate use with
/// async workflows.
///
/// ### Shutting down
///
/// If any of the interfaces returns a `RaftError::Fatal`, this indicates that the Raft node
/// is shutting down. If the parent application needs to shutdown the Raft node for any reason,
/// calling `shutdown` will do the trick.
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
    /// An implementation of the [`RaftNetworkFactory`] trait which will be used by Raft for
    /// sending RPCs to peer nodes within the cluster.
    ///
    /// ### `storage`
    /// An implementation of the [`RaftLogStorage`] and [`RaftStateMachine`] trait which will be
    /// used by Raft for data storage.
    ///
    /// [`RaftNetworkFactory`]: crate::network::RaftNetworkFactory
    #[tracing::instrument(level="debug", skip_all, fields(cluster=%config.cluster_name))]
    pub async fn new<LS, N, SM>(
        id: C::NodeId,
        config: Arc<Config>,
        network: N,
        mut log_store: LS,
        mut state_machine: SM,
    ) -> Result<Self, Fatal<C>>
    where
        N: RaftNetworkFactory<C>,
        LS: RaftLogStorage<C>,
        SM: RaftStateMachine<C>,
    {
        let (tx_api, rx_api) = mpsc::unbounded_channel();
        let (tx_notify, rx_notify) = mpsc::unbounded_channel();
        let (tx_metrics, rx_metrics) = watch::channel(RaftMetrics::new_initial(id));
        let (tx_data_metrics, rx_data_metrics) = watch::channel(RaftDataMetrics::default());
        let (tx_server_metrics, rx_server_metrics) = watch::channel(RaftServerMetrics::default());
        let (tx_shutdown, rx_shutdown) = C::AsyncRuntime::oneshot();

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

        let sm_handle = worker::Worker::spawn(state_machine, tx_notify.clone());

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
            tx_data_metrics,
            tx_server_metrics,

            command_state: CommandState::default(),
            span: core_span,

            chan_log_flushed: LogEventChannel::new(),

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
            rx_data_metrics,
            rx_server_metrics,
            tx_shutdown: Mutex::new(Some(tx_shutdown)),
            core_state: Mutex::new(CoreState::Running(core_handle)),

            snapshot: Mutex::new(None),
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
    /// raft.runtime_config().tick(true);
    /// raft.runtime_config().elect(true);
    /// ```
    pub fn runtime_config(&self) -> RuntimeConfigHandle<C> {
        RuntimeConfigHandle::new(self.inner.as_ref())
    }

    /// Return the config of this Raft node.
    pub fn config(&self) -> &Arc<Config> {
        &self.inner.config
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

    /// Submit an AppendEntries RPC to this Raft node.
    ///
    /// These RPCs are sent by the cluster leader to replicate log entries (§5.3), and are also
    /// used as heartbeats (§5.2).
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub async fn append_entries(&self, rpc: AppendEntriesRequest<C>) -> Result<AppendEntriesResponse<C>, RaftError<C>> {
        tracing::debug!(rpc = display(&rpc), "Raft::append_entries");

        let (tx, rx) = C::AsyncRuntime::oneshot();
        self.inner.call_core(RaftMsg::AppendEntries { rpc, tx }, rx).await
    }

    /// Submit a VoteRequest (RequestVote in the spec) RPC to this Raft node.
    ///
    /// These RPCs are sent by cluster peers which are in candidate state attempting to gather votes
    /// (§5.2).
    #[tracing::instrument(level = "debug", skip(self, rpc))]
    pub async fn vote(&self, rpc: VoteRequest<C>) -> Result<VoteResponse<C>, RaftError<C>> {
        tracing::info!(rpc = display(&rpc), "Raft::vote()");

        let (tx, rx) = C::AsyncRuntime::oneshot();
        self.inner.call_core(RaftMsg::RequestVote { rpc, tx }, rx).await
    }

    /// Get the latest snapshot from the state machine.
    ///
    /// It returns error only when `RaftCore` fails to serve the request, e.g., Encountering a
    /// storage error or shutting down.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_snapshot(&self) -> Result<Option<Snapshot<C>>, RaftError<C>> {
        tracing::debug!("Raft::get_snapshot()");

        let (tx, rx) = C::AsyncRuntime::oneshot();
        let cmd = ExternalCommand::GetSnapshot { tx };
        self.inner.call_core(RaftMsg::ExternalCommand { cmd }, rx).await
    }

    /// Get a snapshot data for receiving snapshot from the leader.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn begin_receiving_snapshot(&self) -> Result<Box<SnapshotDataOf<C>>, RaftError<C, Infallible>> {
        tracing::info!("Raft::begin_receiving_snapshot()");

        let (tx, rx) = AsyncRuntimeOf::<C>::oneshot();
        let resp = self.inner.call_core(RaftMsg::BeginReceivingSnapshot { tx }, rx).await?;
        Ok(resp)
    }

    /// Install a completely received snapshot to the state machine.
    ///
    /// This method is used to implement a totally application defined snapshot transmission.
    /// The application receives a snapshot from the leader, in chunks or a stream, and
    /// then rebuild a snapshot, then pass the snapshot to Raft to install.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn install_full_snapshot(
        &self,
        vote: Vote<C::NodeId>,
        snapshot: Snapshot<C>,
    ) -> Result<SnapshotResponse<C>, Fatal<C>> {
        tracing::info!("Raft::install_full_snapshot()");

        let (tx, rx) = C::AsyncRuntime::oneshot();
        let res = self.inner.call_core(RaftMsg::InstallFullSnapshot { vote, snapshot, tx }, rx).await;
        match res {
            Ok(x) => Ok(x),
            Err(e) => {
                // Safe unwrap: `RaftError<Infallible>` must be a Fatal.
                Err(e.into_fatal().unwrap())
            }
        }
    }

    /// Receive an `InstallSnapshotRequest`.
    ///
    /// These RPCs are sent by the cluster leader in order to bring a new node or a slow node
    /// up-to-speed with the leader.
    ///
    /// If receiving is finished `done == true`, it installs the snapshot to the state machine.
    /// Nothing will be done if the input snapshot is older than the state machine.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn install_snapshot(
        &self,
        req: InstallSnapshotRequest<C>,
    ) -> Result<InstallSnapshotResponse<C>, RaftError<C, crate::error::InstallSnapshotError>>
    where
        C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin,
    {
        tracing::debug!(req = display(&req), "Raft::install_snapshot()");

        let req_vote = req.vote;
        let my_vote = self.with_raft_state(|state| *state.vote_ref()).await?;
        let resp = InstallSnapshotResponse { vote: my_vote };

        // Check vote.
        // It is not mandatory because it is just a read operation
        // but prevent unnecessary snapshot transfer early.
        {
            if req_vote >= my_vote {
                // Ok
            } else {
                tracing::info!("vote {} is rejected by local vote: {}", req_vote, my_vote);
                return Ok(resp);
            }
        }

        let finished_snapshot = {
            use crate::network::snapshot_transport::Chunked;
            use crate::network::snapshot_transport::SnapshotTransport;

            let mut streaming = self.inner.snapshot.lock().await;
            Chunked::receive_snapshot(&mut *streaming, self, req).await?
        };

        if let Some(snapshot) = finished_snapshot {
            let resp = self.install_full_snapshot(req_vote, snapshot).await?;
            return Ok(resp.into());
        }
        Ok(resp)
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
    #[deprecated(since = "0.9.0", note = "use `Raft::ensure_linearizable()` instead")]
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn is_leader(&self) -> Result<(), RaftError<C, CheckIsLeaderError<C>>> {
        let (tx, rx) = C::AsyncRuntime::oneshot();
        let _ = self.inner.call_core(RaftMsg::CheckIsLeaderRequest { tx }, rx).await?;
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
    pub async fn ensure_linearizable(&self) -> Result<Option<LogId<C::NodeId>>, RaftError<C, CheckIsLeaderError<C>>> {
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
    ) -> Result<(Option<LogId<C::NodeId>>, Option<LogId<C::NodeId>>), RaftError<C, CheckIsLeaderError<C>>> {
        let (tx, rx) = C::AsyncRuntime::oneshot();
        let (read_log_id, applied) = self.inner.call_core(RaftMsg::CheckIsLeaderRequest { tx }, rx).await?;
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
    /// [`RaftStateMachine::apply`] method is the perfect place to implement
    /// this.
    ///
    /// These are application specific requirements, and must be implemented by the application
    /// which is being built on top of Raft.
    #[tracing::instrument(level = "debug", skip(self, app_data))]
    pub async fn client_write<E>(
        &self,
        app_data: C::D,
    ) -> Result<ClientWriteResponse<C>, RaftError<C, ClientWriteError<C>>>
    where
        ResponderReceiverOf<C>: Future<Output = Result<ClientWriteResult<C>, E>>,
        E: Error + OptionalSend,
    {
        let rx = self.client_write_ff(app_data).await?;

        let res: ClientWriteResult<C> = self.inner.recv_msg(rx).await?;

        let client_write_response = res.map_err(|e| RaftError::APIError(e))?;
        Ok(client_write_response)
    }

    /// Submit a mutating client request to Raft to update the state machine, returns an application
    /// defined response receiver [`Responder::Receiver`].
    ///
    /// `_ff` means fire and forget.
    ///
    /// It is same as [`Raft::client_write`] but does not wait for the response.
    #[tracing::instrument(level = "debug", skip(self, app_data))]
    pub async fn client_write_ff(&self, app_data: C::D) -> Result<ResponderReceiverOf<C>, Fatal<C>> {
        let (app_data, tx, rx) = ResponderOf::<C>::from_app_data(app_data);

        self.inner.send_msg(RaftMsg::ClientWriteRequest { app_data, tx }).await?;

        Ok(rx)
    }

    /// Return `true` if this node is already initialized and can not be initialized again with
    /// [`Raft::initialize`]
    pub async fn is_initialized(&self) -> Result<bool, Fatal<C>> {
        let initialized = self.with_raft_state(|st| st.is_initialized()).await?;

        Ok(initialized)
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
    pub async fn initialize<T>(&self, members: T) -> Result<(), RaftError<C, InitializeError<C>>>
    where T: IntoNodes<C::NodeId, C::Node> + Debug {
        let (tx, rx) = C::AsyncRuntime::oneshot();
        self.inner
            .call_core(
                RaftMsg::Initialize {
                    members: members.into_nodes(),
                    tx,
                },
                rx,
            )
            .await
    }

    /// Returns Ok() with the latest known matched log id if it should quit waiting: leader change,
    /// node removed, or replication becomes upto date.
    ///
    /// Returns Err() if it should keep waiting.
    fn check_replication_upto_date(
        &self,
        metrics: &RaftMetrics<C>,
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

    /// Provides read-only access to [`RaftState`] through a user-provided function.
    ///
    /// The function `func` is applied to the current [`RaftState`]. The result of this function,
    /// of type `V`, is returned wrapped in `Result<V, Fatal<C>>`. `Fatal` error will be
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
    pub async fn with_raft_state<F, V>(&self, func: F) -> Result<V, Fatal<C>>
    where
        F: FnOnce(&RaftState<C>) -> V + OptionalSend + 'static,
        V: OptionalSend + 'static,
    {
        let (tx, rx) = C::AsyncRuntime::oneshot();

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
    where F: FnOnce(&RaftState<C>) + OptionalSend + 'static {
        let req: BoxCoreFn<C> = Box::new(req);
        let _ignore_error = self.inner.tx_api.send(RaftMsg::ExternalCoreRequest { req });
    }

    /// Get a handle to the metrics channel.
    pub fn metrics(&self) -> watch::Receiver<RaftMetrics<C>> {
        self.inner.rx_metrics.clone()
    }

    /// Get a handle to the data metrics channel.
    pub fn data_metrics(&self) -> watch::Receiver<RaftDataMetrics<C>> {
        self.inner.rx_data_metrics.clone()
    }

    /// Get a handle to the server metrics channel.
    pub fn server_metrics(&self) -> watch::Receiver<RaftServerMetrics<C>> {
        self.inner.rx_server_metrics.clone()
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
    pub fn wait(&self, timeout: Option<Duration>) -> Wait<C> {
        let timeout = match timeout {
            Some(t) => t,
            None => Duration::from_secs(86400 * 365 * 100),
        };
        Wait {
            timeout,
            rx: self.inner.rx_metrics.clone(),
        }
    }

    /// Shutdown this Raft node.
    ///
    /// It sends a shutdown signal and waits until `RaftCore` returns.
    pub async fn shutdown(&self) -> Result<(), JoinErrorOf<C>> {
        if let Some(tx) = self.inner.tx_shutdown.lock().await.take() {
            // A failure to send means the RaftCore is already shutdown. Continue to check the task
            // return value.
            let send_res = tx.send(());
            tracing::info!("sending shutdown signal to RaftCore, sending res: {:?}", send_res);
        }
        self.inner.join_core_task().await;
        if let Some(join_handle) = self.inner.tick_handle.shutdown() {
            let _ = join_handle.await;
        }

        // TODO(xp): API change: replace `JoinError` with `Fatal`,
        //           to let the caller know the return value of RaftCore task.
        Ok(())
    }
}
