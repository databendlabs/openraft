//! Public Raft interface and data types.
//!
//! [`Raft`] serves as the primary interface to a Raft node,
//! facilitating all interactions with the underlying RaftCore.
//!
//! While `RaftCore` operates as a singleton within an application, [`Raft`] instances are designed
//! to be cheaply cloneable.
//! This allows multiple components within the application that require interaction with `RaftCore`
//! to efficiently share access.

pub(crate) mod api;
#[cfg(test)]
mod declare_raft_types_test;
mod impl_raft_blocking_write;
pub(crate) mod message;
mod raft_inner;
pub mod responder;
mod runtime_config_handle;
pub mod trigger;

use std::collections::BTreeMap;
use std::error::Error;

pub(crate) use api::app::AppApi;
pub(crate) use api::management::ManagementApi;
pub(crate) use api::protocol::ProtocolApi;

pub(in crate::raft) mod core_state;

use std::fmt::Debug;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use core_state::CoreState;
use derive_more::Display;
pub use message::AppendEntriesRequest;
pub use message::AppendEntriesResponse;
pub use message::ClientWriteResponse;
pub use message::ClientWriteResult;
pub use message::InstallSnapshotRequest;
pub use message::InstallSnapshotResponse;
pub use message::SnapshotResponse;
pub use message::TransferLeaderRequest;
pub use message::VoteRequest;
pub use message::VoteResponse;
use openraft_macros::since;
use tracing::trace_span;
use tracing::Instrument;
use tracing::Level;

use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::MpscUnboundedSender;
use crate::async_runtime::OneshotSender;
use crate::base::BoxAsyncOnceMut;
use crate::base::BoxFuture;
use crate::base::BoxOnce;
use crate::config::Config;
use crate::config::RuntimeConfig;
use crate::core::heartbeat::handle::HeartbeatWorkersHandle;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::core::raft_msg::RaftMsg;
use crate::core::sm;
use crate::core::sm::worker;
use crate::core::RaftCore;
use crate::core::Tick;
use crate::engine::Engine;
use crate::engine::EngineConfig;
use crate::error::into_raft_result::IntoRaftResult;
use crate::error::CheckIsLeaderError;
use crate::error::ClientWriteError;
use crate::error::Fatal;
use crate::error::InitializeError;
use crate::error::InvalidStateMachineType;
use crate::error::RaftError;
use crate::membership::IntoNodes;
use crate::metrics::RaftDataMetrics;
use crate::metrics::RaftMetrics;
use crate::metrics::RaftServerMetrics;
use crate::metrics::Wait;
use crate::raft::raft_inner::RaftInner;
pub use crate::raft::runtime_config_handle::RuntimeConfigHandle;
use crate::raft::trigger::Trigger;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::storage::Snapshot;
use crate::type_config::alias::JoinErrorOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::ResponderReceiverOf;
use crate::type_config::alias::SnapshotDataOf;
use crate::type_config::alias::VoteOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::TypeConfigExt;
use crate::vote::raft_vote::RaftVoteExt;
use crate::OptionalSend;
use crate::RaftNetworkFactory;
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
///    pub TypeConfig:
///        D            = ClientRequest,
///        R            = ClientResponse,
///        NodeId       = u64,
///        Node         = openraft::BasicNode,
///        Term         = u64,
///        LeaderId     = openraft::impls::leader_id_adv::LeaderId<Self>,
///        Vote         = openraft::impls::Vote<Self>,
///        Entry        = openraft::Entry<Self>,
///        SnapshotData = Cursor<Vec<u8>>,
///        Responder    = openraft::impls::OneshotResponder<Self>,
///        AsyncRuntime = openraft::TokioRuntime,
/// );
/// ```
///
/// Types can be omitted, and the following default type will be used:
/// - `D`:            `String`
/// - `R`:            `String`
/// - `NodeId`:       `u64`
/// - `Node`:         `::openraft::impls::BasicNode`
/// - `Term`:         `u64`
/// - `LeaderId`:     `::openraft::impls::leader_id_adv::LeaderId<Self>`
/// - `Vote`:         `::openraft::impls::Vote<Self>`
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
                (D            , , String                                       ),
                (R            , , String                                       ),
                (NodeId       , , u64                                          ),
                (Node         , , $crate::impls::BasicNode                     ),
                (Term         , , u64                                          ),
                (LeaderId     , , $crate::impls::leader_id_adv::LeaderId<Self> ),
                (Vote         , , $crate::impls::Vote<Self>                    ),
                (Entry        , , $crate::impls::Entry<Self>                   ),
                (SnapshotData , , std::io::Cursor<Vec<u8>>                     ),
                (Responder    , , $crate::impls::OneshotResponder<Self>        ),
                (AsyncRuntime , , $crate::impls::TokioRuntime                  ),
            );

        }
    };
}

/// Policy that determines how to handle read operations in a Raft cluster.
///
/// This enum defines strategies for ensuring linearizable reads in distributed systems
/// while balancing between consistency guarantees and performance.
#[derive(Clone, Debug, Display, PartialEq, Eq)]
pub enum ReadPolicy {
    /// Uses leader lease to avoid network round-trips for read operations.
    ///
    /// With `LeaseRead`, the leader can serve reads locally without contacting followers
    /// as long as it believes its leadership lease is still valid. This provides better
    /// performance compared to `ReadIndex` but assumes clock drift between nodes is negligible.
    ///
    /// Note: This offers slightly weaker consistency guarantees than `ReadIndex` in exchange
    /// for lower latency.
    LeaseRead,

    /// Implements the ReadIndex protocol to ensure linearizable reads.
    ///
    /// With `ReadIndex`, the leader confirms its leadership status by contacting a quorum
    /// of followers before serving read requests. This ensures strong consistency but incurs
    /// the cost of network communication for each read operation.
    ///
    /// This is the safer option that provides the strongest consistency guarantees.
    ReadIndex,
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
        let (tx_api, rx_api) = C::mpsc_unbounded();
        let (tx_notify, rx_notify) = C::mpsc_unbounded();
        let (tx_metrics, rx_metrics) = C::watch_channel(RaftMetrics::new_initial(id.clone()));
        let (tx_data_metrics, rx_data_metrics) = C::watch_channel(RaftDataMetrics::default());
        let (tx_server_metrics, rx_server_metrics) = C::watch_channel(RaftServerMetrics::default());
        let (tx_shutdown, rx_shutdown) = C::oneshot();

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
            id = display(&id),
            cluster = display(&config.cluster_name)
        );

        let eng_config = EngineConfig::new(id.clone(), config.as_ref());

        let state = {
            let mut helper = StorageHelper::new(&mut log_store, &mut state_machine);
            helper.get_initial_state().await?
        };

        let engine = Engine::new(state, eng_config);

        let sm_span = tracing::span!(parent: &core_span, Level::DEBUG, "sm_worker");

        let sm_handle = worker::Worker::spawn(
            state_machine,
            log_store.get_log_reader().await,
            tx_notify.clone(),
            sm_span,
        );

        let core: RaftCore<C, N, LS> = RaftCore {
            id: id.clone(),
            config: config.clone(),
            runtime_config: runtime_config.clone(),
            network_factory: network,
            log_store,
            sm_handle,

            engine,

            client_resp_channels: BTreeMap::new(),

            replications: Default::default(),

            heartbeat_handle: HeartbeatWorkersHandle::new(id.clone(), config.clone()),
            tx_api: tx_api.clone(),
            rx_api,

            tx_notification: tx_notify,
            rx_notification: rx_notify,

            tx_metrics,
            tx_data_metrics,
            tx_server_metrics,

            span: core_span,
        };

        let core_handle = C::spawn(core.main(rx_shutdown).instrument(trace_span!("spawn").or_current()));

        let inner = RaftInner {
            id,
            config,
            runtime_config,
            tick_handle,
            tx_api,
            rx_metrics,
            rx_data_metrics,
            rx_server_metrics,
            tx_shutdown: std::sync::Mutex::new(Some(tx_shutdown)),
            core_state: std::sync::Mutex::new(CoreState::Running(core_handle)),

            snapshot: C::mutex(None),
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

    /// Create a new [`ProtocolApi`] to handle Raft protocal RPCs received by this Raft node.
    ///
    /// [`ProtocolApi`] provides the following protocol APIs:
    /// - [`ProtocolApi::append_entries`]
    /// - [`ProtocolApi::vote`]
    /// - [`ProtocolApi::get_snapshot`]
    /// - [`ProtocolApi::begin_receiving_snapshot`]
    /// - [`ProtocolApi::install_full_snapshot`]
    /// - [`ProtocolApi::handle_transfer_leader`]
    pub(crate) fn protocol_api(&self) -> ProtocolApi<C> {
        ProtocolApi::new(self.inner.as_ref())
    }

    pub(crate) fn app_api(&self) -> AppApi<C> {
        AppApi::new(self.inner.as_ref())
    }

    pub(crate) fn management_api(&self) -> ManagementApi<C> {
        ManagementApi::new(self.inner.as_ref())
    }

    /// Return a [`Trigger`] handle to manually trigger raft actions, such as elect or build
    /// snapshot.
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
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn append_entries(&self, rpc: AppendEntriesRequest<C>) -> Result<AppendEntriesResponse<C>, RaftError<C>> {
        self.protocol_api().append_entries(rpc).await.into_raft_result()
    }

    /// Submit a VoteRequest (RequestVote in the spec) RPC to this Raft node.
    ///
    /// These RPCs are sent by cluster peers which are in candidate state attempting to gather votes
    /// (§5.2).
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn vote(&self, rpc: VoteRequest<C>) -> Result<VoteResponse<C>, RaftError<C>> {
        self.protocol_api().vote(rpc).await.into_raft_result()
    }

    /// Get the latest snapshot from the state machine.
    ///
    /// It returns error only when `RaftCore` fails to serve the request, e.g., Encountering a
    /// storage error or shutting down.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_snapshot(&self) -> Result<Option<Snapshot<C>>, RaftError<C>> {
        self.protocol_api().get_snapshot().await.into_raft_result()
    }

    /// Get a snapshot data for receiving snapshot from the leader.
    #[since(version = "0.10.0", change = "SnapshotData without Box")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn begin_receiving_snapshot(&self) -> Result<SnapshotDataOf<C>, RaftError<C>> {
        self.protocol_api().begin_receiving_snapshot().await.into_raft_result()
    }

    /// Install a completely received snapshot to the state machine.
    ///
    /// This method is used to implement an application defined snapshot transmission.
    /// The application receives a snapshot from the leader, in chunks or a stream, and
    /// then rebuild a snapshot, then pass the snapshot to Raft to install.
    #[since(version = "0.9.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn install_full_snapshot(
        &self,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
    ) -> Result<SnapshotResponse<C>, Fatal<C>> {
        self.protocol_api().install_full_snapshot(vote, snapshot).await
    }

    /// Receive an `InstallSnapshotRequest`.
    ///
    /// These RPCs are sent by the cluster leader in order to bring a new node or a slow node
    /// up-to-speed with the leader.
    ///
    /// If receiving is finished `done == true`, it installs the snapshot to the state machine.
    /// Nothing will be done if the input snapshot is older than the state machine.
    #[tracing::instrument(level = "debug", skip_all)]
    #[cfg(feature = "tokio-rt")]
    pub async fn install_snapshot(
        &self,
        req: InstallSnapshotRequest<C>,
    ) -> Result<InstallSnapshotResponse<C>, RaftError<C, crate::error::InstallSnapshotError>>
    where
        C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin,
    {
        use crate::async_runtime::mutex::Mutex;

        tracing::debug!(req = display(&req), "Raft::install_snapshot()");

        let req_vote = req.vote.clone();
        let my_vote = self.with_raft_state(|state| state.vote_ref().clone()).await?;
        let resp = InstallSnapshotResponse { vote: my_vote.clone() };

        // Check vote.
        // It is not mandatory because it is just a read operation
        // but prevent unnecessary snapshot transfer early.
        {
            if req_vote.as_ref_vote() >= my_vote.as_ref_vote() {
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
        self.metrics().borrow_watched().current_leader.clone()
    }

    /// Ensures reads performed after this method are linearizable across the cluster
    /// using an explicitly provided policy.
    ///
    /// **NOTE**: Calling this method on a non-leader node will get a `ForwardToLeader` error.
    /// If you want to do linearizable read on a non-leader node, you can fire a rpc request
    /// `get_read_log_id` from leader, Then call [`wait_apply()`](Raft::wait_apply) to wait for
    /// the state machine to apply to the log id you specified.
    ///
    /// This method is just a shorthand for calling [`get_read_log_id()`](Raft::get_read_log_id) and
    /// then calling [Raft::wait].
    ///
    /// The policy can be one of:
    /// - `ReadPolicy::ReadIndex`: Provides strongest consistency guarantees by confirming
    ///   leadership with a quorum before serving reads, but incurs higher latency due to network
    ///   communication.
    /// - `ReadPolicy::LeaseRead`: Uses leadership lease to avoid network round-trips, providing
    ///   better performance but slightly weaker consistency guarantees (assumes minimal clock drift
    ///   between nodes).
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
    /// // Use a strict policy for this specific critical read
    /// my_raft.ensure_linearizable_with_policy(ReadPolicy::ReadIndex).await?;
    /// // Or use a more performant policy when consistency requirements are less strict
    /// my_raft.ensure_linearizable_with_policy(ReadPolicy::LeaseRead).await?;
    /// // Then proceed with the state machine read
    /// ```
    /// Read more about how it works: [Read Operation](crate::docs::protocol::read)
    #[since(version = "0.9.0")]
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn ensure_linearizable(
        &self,
        read_policy: ReadPolicy,
    ) -> Result<Option<LogIdOf<C>>, RaftError<C, CheckIsLeaderError<C>>> {
        self.app_api().ensure_linearizable(read_policy).await.into_raft_result()
    }

    /// Ensures this node is leader and returns the log id up to which the state machine should
    /// apply to ensure a read can be linearizable across the cluster using an explicitly provided
    /// policy.
    ///
    /// The leadership is ensured by sending heartbeats to a quorum of followers.
    /// Note that this is just the first step for linearizable read. The second step is to wait for
    /// state machine to reach the returned `read_log_id`.
    ///
    /// The policy can be one of:
    /// - `ReadPolicy::ReadIndex`: Provides strongest consistency guarantees by confirming
    ///   leadership with a quorum before serving reads, but incurs higher latency due to network
    ///   communication.
    /// - `ReadPolicy::LeaseRead`: Uses leadership lease to avoid network round-trips, providing
    ///   better performance but slightly weaker consistency guarantees (assumes minimal clock drift
    ///   between nodes).
    ///
    /// Returns:
    /// - `Ok((read_log_id, last_applied_log_id))` on successful confirmation that the node is the
    ///   leader. `read_log_id` represents the log id up to which the state machine should apply to
    ///   ensure a linearizable read.
    /// - `Err(RaftError<CheckIsLeaderError>)` if this node fail to ensure its leadership, for
    ///   example, it detects a higher term, or fails to communicate with a quorum.
    ///
    /// Once returned, the caller should wait for the state machine to apply entries up to
    /// `read_log_id`. This can be done by using the [`wait()`] as shown in the example
    /// below, or by directly monitoring `last_applied` in [`Raft::metrics`] until it reaches or
    /// exceeds `read_log_id`.
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
    ///
    /// [`wait()`]: Raft::wait
    #[since(version = "0.9.0")]
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_read_log_id(
        &self,
        read_policy: ReadPolicy,
    ) -> Result<(Option<LogIdOf<C>>, Option<LogIdOf<C>>), RaftError<C, CheckIsLeaderError<C>>> {
        self.app_api().get_read_log_id(read_policy).await.into_raft_result()
    }

    /// `wait_apply` wait for the state machine to apply entries up to
    /// `read_log_id`. The `applied` should be current node's `last_applied`.
    ///
    /// If `timeout` is `None`, then it will wait forever(10 years).
    /// If `timeout` is `Some`, then it will wait for the specified duration.
    #[since(version = "0.11.0")]
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn wait_apply(
        &self,
        read_log_id: Option<LogIdOf<C>>,
        applied: Option<LogIdOf<C>>,
        timeout: Option<Duration>,
    ) -> Result<Option<LogIdOf<C>>, RaftError<C>> {
        self.app_api().wait_apply(read_log_id, applied, timeout).await.into_raft_result()
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
        self.app_api().client_write(app_data).await.into_raft_result()
    }

    /// Submit a mutating client request to Raft to update the state machine, returns an application
    /// defined response receiver [`Responder::Receiver`].
    ///
    /// `_ff` means fire and forget.
    ///
    /// It is same as [`Self::client_write`] but does not wait for the response.
    #[since(version = "0.10.0")]
    pub async fn client_write_ff(&self, app_data: C::D) -> Result<ResponderReceiverOf<C>, Fatal<C>> {
        self.app_api().client_write_ff(app_data).await
    }

    /// Handle the LeaderTransfer request from a Leader node.
    ///
    /// If this node is the `to` node, it resets the Leader lease and triggers an election when the
    /// expected log entries are flushed.
    /// Otherwise, it just resets the Leader lease to allow the `to` node to become the Leader.
    ///
    /// The application calls
    /// [`Raft::trigger().transfer_leader()`](crate::raft::trigger::Trigger::transfer_leader) to
    /// submit Transfer Leader command. Then, the current Leader will broadcast it to every node in
    /// the cluster via [`RaftNetworkV2::transfer_leader`] and the implementation on the remote node
    /// responds to transfer leader request by calling this method.
    ///
    /// [`RaftNetworkV2::transfer_leader`]: crate::network::v2::RaftNetworkV2::transfer_leader
    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn handle_transfer_leader(&self, req: TransferLeaderRequest<C>) -> Result<(), Fatal<C>> {
        self.protocol_api().handle_transfer_leader(req).await
    }

    /// Return `true` if this node is already initialized and can not be initialized again with
    /// [`Raft::initialize`]
    #[since(version = "0.10.0")]
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
    /// already up and running, which is ultimately the goal of this function. You can check
    /// if the cluster is initialized with [`Raft::is_initialized()`] and then avoid re-initialize
    /// it in case you want to get rid of this error.
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
        self.management_api().initialize(members).await.into_raft_result()
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
        let (tx, rx) = C::oneshot();

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
                let fatal = self.inner.get_core_stop_error().await;
                Err(fatal)
            }
        }
    }

    /// Send a request to the Raft core loop in a fire-and-forget manner.
    ///
    /// The request functor will be called with an immutable reference to the [`RaftState`]
    /// and serialized with other Raft core loop processing (e.g., client requests
    /// or general state changes).
    ///
    /// If a response is required, then the caller can store the sender of a one-shot channel
    /// in the closure of the request functor, which can then be used to send the response
    /// asynchronously.
    ///
    /// If the API channel is already closed (Raft is in shutdown), then the request functor is
    /// destroyed right away and not called at all.
    pub fn external_request<F>(&self, req: F)
    where F: FnOnce(&RaftState<C>) + OptionalSend + 'static {
        let req: BoxOnce<'static, RaftState<C>> = Box::new(req);
        let _ignore_error = self.inner.tx_api.send(RaftMsg::ExternalCoreRequest { req });
    }

    /// Provides mutable access to [`RaftStateMachine`] through a user-provided function.
    ///
    /// The function `func` is applied to the current [`RaftStateMachine`]. The result of this
    /// function, of type `V`, is returned wrapped in
    /// `Result<Result<V, InvalidStateMachineType>, Fatal<C>>`.
    /// `Fatal` error will be returned if failed to receive a reply from `RaftCore`.
    ///
    /// A `Fatal` error is returned if:
    /// - Raft core task is stopped normally.
    /// - Raft core task is panicked due to programming error.
    /// - Raft core task is encountered a storage error.
    ///
    /// If the user function fail to run, e.g., the input `SM` is different one from the one in
    /// `RaftCore`, it returns an [`InvalidStateMachineType`] error.
    ///
    /// Example for getting the last applied log id from SM(assume there is `last_applied()` method
    /// provided):
    ///
    /// ```rust,ignore
    /// let last_applied_log_id = my_raft.with_state_machine(|sm| {
    ///     async move { sm.last_applied().await }
    /// }).await?;
    /// ```
    #[since(version = "0.10.0")]
    pub async fn with_state_machine<F, SM, V>(&self, func: F) -> Result<Result<V, InvalidStateMachineType>, Fatal<C>>
    where
        SM: RaftStateMachine<C>,
        F: FnOnce(&mut SM) -> BoxFuture<V> + OptionalSend + 'static,
        V: OptionalSend + 'static,
    {
        let (tx, rx) = C::oneshot();

        self.external_state_machine_request(|sm| {
            Box::pin(async move {
                let resp = func(sm).await;
                if let Err(_err) = tx.send(resp) {
                    tracing::error!("{}: fail to send response to user communicating tx", func_name!());
                }
            })
        });

        let recv_res = rx.await;
        tracing::debug!("{} receives result is error: {:?}", func_name!(), recv_res.is_err());

        let Ok(v) = recv_res else {
            if self.inner.is_core_running() {
                return Ok(Err(InvalidStateMachineType::new::<SM>()));
            } else {
                let fatal = self.inner.get_core_stop_error().await;
                tracing::error!(error = debug(&fatal), "error when {}", func_name!());
                return Err(fatal);
            }
        };

        Ok(Ok(v))
    }

    /// Send a request to the [`RaftStateMachine`] worker in a fire-and-forget manner.
    ///
    /// The request functor will be called with a mutable reference to the state machine.
    /// The functor returns a [`Future`] because state machine methods are `async`.
    ///
    /// If the API channel is already closed (Raft is in shutdown), then the request functor is
    /// destroyed right away and not called at all.
    ///
    /// If the input `SM` is different from the one in `RaftCore`, it just silently ignores it.
    #[since(version = "0.10.0")]
    pub fn external_state_machine_request<F, SM>(&self, req: F)
    where
        SM: RaftStateMachine<C>,
        F: FnOnce(&mut SM) -> BoxFuture<()> + OptionalSend + 'static,
    {
        let input_sm_type = std::any::type_name::<SM>();

        let func: BoxAsyncOnceMut<'static, SM> = Box::new(req);

        // Erase the type so that to send through a channel without `SM` type parameter.
        // `sm::Worker` will downcast it back to BoxAsyncOnce<SM>.
        let func = Box::new(func);

        let sm_cmd = sm::Command::Func { func, input_sm_type };
        let raft_msg = RaftMsg::ExternalCommand {
            cmd: ExternalCommand::StateMachineCommand { sm_cmd },
        };
        let _ignore_error = self.inner.tx_api.send(raft_msg);
    }

    /// Get a handle to the metrics channel.
    pub fn metrics(&self) -> WatchReceiverOf<C, RaftMetrics<C>> {
        self.inner.rx_metrics.clone()
    }

    /// Get a handle to the data metrics channel.
    pub fn data_metrics(&self) -> WatchReceiverOf<C, RaftDataMetrics<C>> {
        self.inner.rx_data_metrics.clone()
    }

    /// Get a handle to the server metrics channel.
    pub fn server_metrics(&self) -> WatchReceiverOf<C, RaftServerMetrics<C>> {
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
        self.inner.wait(timeout)
    }

    /// Shutdown this Raft node.
    ///
    /// It sends a shutdown signal and waits until `RaftCore` returns.
    pub async fn shutdown(&self) -> Result<(), JoinErrorOf<C>> {
        if let Some(tx) = self.inner.tx_shutdown.lock().unwrap().take() {
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
