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
mod impl_raft_client;
mod impl_raft_lifecycle;
mod impl_raft_protocol;
mod impl_raft_watch;
pub mod linearizable_read;
pub(crate) mod message;
mod raft_inner;
pub mod responder;
mod runtime_config_handle;
pub(crate) mod stream_append;
pub mod trigger;
mod watch_handle;

pub(crate) use api::app::AppApi;
pub(crate) use api::management::ManagementApi;
pub(crate) use api::protocol::ProtocolApi;

pub(in crate::raft) mod core_state;
mod leader;

use std::fmt::Debug;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use core_state::CoreState;
use derive_more::Display;
use futures_util::FutureExt;
pub use message::AppendEntriesRequest;
pub use message::AppendEntriesResponse;
pub use message::ClientWriteResponse;
pub use message::ClientWriteResult;
pub use message::InstallSnapshotRequest;
pub use message::InstallSnapshotResponse;
pub use message::LogSegment;
pub use message::SnapshotResponse;
pub use message::StreamAppendError;
pub use message::TransferLeaderError;
pub use message::TransferLeaderRequest;
pub use message::TransferLeaderResponse;
pub use message::VoteRequest;
pub use message::VoteResponse;
pub use message::WriteRequest;
pub use message::WriteResponse;
pub use message::WriteResult;
use openraft_macros::since;
pub use stream_append::StreamAppendResult;
use tracing::Instrument;
use tracing::Level;
use tracing::trace_span;

pub use self::leader::Leader;
pub use self::watch_handle::WatchChangeHandle;
use crate::Extensions;
use crate::OptionalSend;
use crate::RaftNetworkFactory;
pub use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StorageHelper;
use crate::async_runtime::MpscWeakSender;
use crate::async_runtime::mpsc::MpscSender;
use crate::async_runtime::watch::WatchReceiver;
use crate::config::Config;
use crate::config::RuntimeConfig;
use crate::core::ClientResponderQueue;
use crate::core::RaftCore;
use crate::core::SharedReplicateBatch;
use crate::core::StepDownWatcher;
use crate::core::Tick;
use crate::core::heartbeat::handle::HeartbeatWorkersHandle;
pub use crate::core::io_flush_tracking::FlushPoint;
use crate::core::io_flush_tracking::IoProgressWatcher;
use crate::core::merged_raft_msg_receiver::BatchRaftMsgReceiver;
use crate::core::notification::Notification;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::core::raft_msg::install_full_snapshot_request::InstallFullSnapshotRequest;
use crate::core::runtime_stats::RuntimeStats;
use crate::core::sm;
use crate::core::sm::worker;
use crate::engine::Engine;
use crate::engine::EngineConfig;
use crate::errors::Fatal;
use crate::errors::ForwardToLeader;
use crate::metrics::MetricsRecorder;
use crate::metrics::RaftDataMetrics;
use crate::metrics::RaftMetrics;
use crate::metrics::RaftServerMetrics;
use crate::network::NetSnapshot;
use crate::raft::raft_inner::RaftInner;
pub use crate::raft::runtime_config_handle::RuntimeConfigHandle;
use crate::raft::trigger::Trigger;
use crate::raft_state::IOId;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::MpscWeakSenderOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::vote::leader_id::raft_leader_id::RaftLeaderId;
use crate::vote::leader_id::raft_leader_id::RaftLeaderIdExt;
use crate::vote::non_committed::UncommittedVote;
use crate::vote::raft_vote::RaftVote;
use crate::vote::raft_vote::RaftVoteExt;

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
///        LeaderId     = openraft::impls::leader_id_adv::LeaderId<Self::Term, Self::NodeId>,
///        Vote           = openraft::impls::Vote<Self::LeaderId>,
///        Entry          = openraft::Entry<Self>,
///        Responder<T>   = openraft::impls::OneshotResponder<Self, T>,
///        AsyncRuntime   = openraft::TokioRuntime,
/// );
/// ```
///
/// Types can be omitted, and the following default type will be used:
/// - `D`:            `String`
/// - `R`:            `String`
/// - `NodeId`:       `u64`
/// - `Node`:         `::openraft::impls::BasicNode`
/// - `Term`:         `u64`
/// - `LeaderId`:     `::openraft::impls::leader_id_adv::LeaderId<Self::Term, Self::NodeId>`
/// - `Vote`:           `::openraft::impls::Vote<Self::LeaderId>`
/// - `Entry`:          `::openraft::impls::Entry<Self>`
/// - `Responder<T>`:   `::openraft::impls::OneshotResponder<Self, T>`
/// - `AsyncRuntime`:   `::openraft::impls::TokioRuntime`
/// - `ErrorSource`:    `::anyerror::AnyError`
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
            $crate::macros::expand!(
                KEYED,
                (T, ATTR, V) => {ATTR type T = V;},
                $(($type_id, $(#[$inner])*, $type),)*

                // Default types:
                (D            , , String                                       ),
                (R            , , String                                       ),
                (NodeId       , , u64                                          ),
                (Node         , , $crate::impls::BasicNode                     ),
                (Term         , , u64                                          ),
                (LeaderId     , , $crate::impls::leader_id_adv::LeaderId<Self::Term, Self::NodeId> ),
                (Vote           , , $crate::impls::Vote<Self::LeaderId>            ),
                (Entry          , , $crate::Entry<<Self::LeaderId as $crate::vote::RaftLeaderId>::Committed, Self::D, Self::NodeId, Self::Node> ),
                (Responder<T>   , , $crate::impls::ProgressResponder<Self, T> where T: $crate::OptionalSend + 'static     ),
                (Batch<T>       , , $crate::impls::InlineBatch<T> where T: $crate::OptionalSend + 'static     ),
                (AsyncRuntime   , , $crate::impls::TokioRuntime                  ),
                (ErrorSource    , , $crate::impls::BoxedErrorSource               ),
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

/// Primary interface to a Raft node.
///
/// `Raft` provides the complete implementation of the Raft consensus protocol and serves as the
/// main interface for interacting with a Raft node in the cluster. Applications built on Raft use
/// this type to spawn a Raft task and communicate with it.
///
/// # Architecture
///
/// The `Raft` handle is a lightweight wrapper around an `Arc<RaftInner>`, making it cheap to clone.
/// The actual work is performed by an internal core task, which runs separately processing
/// requests through message channels.
///
/// # Lifecycle
///
/// 1. **Creation**: Use [`Raft::new`] to create and spawn a new Raft node
/// 2. **Initialization**: Call [`initialize`](Raft::initialize) on pristine nodes to form a cluster
/// 3. **Operation**: Use various methods to interact with the node:
///    - Protocol RPCs: [`append_entries`](Raft::append_entries), [`vote`](Raft::vote)
///    - Client operations: [`client_write`](Raft::client_write),
///      [`ensure_linearizable`](Raft::ensure_linearizable)
///    - Management: [`trigger`](Raft::trigger), [`metrics`](Raft::metrics)
/// 4. **Shutdown**: Call [`shutdown`](Raft::shutdown) to gracefully stop the node
///
/// # Cloning
///
/// `Raft` implements [`Clone`] with very low cost, allowing multiple components in your application
/// to hold handles to the same Raft node. All clones reference the same underlying Raft instance.
///
/// # Error Handling
///
/// Methods return [`RaftError::Fatal`] when the Raft node encounters unrecoverable errors or is
/// shutting down. Applications should monitor for fatal errors and initiate shutdown if needed.
///
/// # Examples
///
/// ```ignore
/// // Create a new Raft node
/// let raft = Raft::new(node_id, config, network, log_store, state_machine).await?;
///
/// // Initialize a new cluster
/// raft.initialize(btreeset![1, 2, 3]).await?;
///
/// // Write to the cluster
/// let response = raft.client_write(my_request).await?;
///
/// // Read linearizably
/// raft.ensure_linearizable(ReadPolicy::ReadIndex).await?;
/// let data = raft.with_state_machine(|sm| { sm.read("key") }).await?;
///
/// // Monitor metrics
/// let metrics = raft.metrics().borrow_watched();
/// println!("Current leader: {:?}", metrics.current_leader);
/// ```
///
/// # See Also
///
/// - [Raft specification](https://raft.github.io/raft.pdf) for protocol details
/// - [`Config`] for configuration options
/// - [`RaftMetrics`] for monitoring cluster state
#[since(version = "0.10.0", change = "added SM state machine type parameter")]
pub struct Raft<C, SM = ()>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    inner: Arc<RaftInner<C>>,
    sm_cmd_tx: MpscWeakSenderOf<C, sm::Command<C, SM>>,

    /// Sender of the dedicated channel that delivers a full snapshot to RaftCore.
    ///
    /// The snapshot data type is defined by the state machine, thus it does not go through
    /// the [`RaftMsg`] channel, which is independent of the state machine type.
    ///
    /// [`RaftMsg`]: crate::core::raft_msg::RaftMsg
    install_snapshot_tx: MpscSenderOf<C, InstallFullSnapshotRequest<C, SM>>,
}

impl<C, SM> Clone for Raft<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            sm_cmd_tx: self.sm_cmd_tx.clone(),
            install_snapshot_tx: self.install_snapshot_tx.clone(),
        }
    }
}

impl<C, SM> Debug for Raft<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Raft").field("id", &self.inner.id).finish()
    }
}

/// Forwarder task that bridges IO completion Watch channel to notification channel.
///
/// This task reads IO completion results from a Watch channel and forwards them
/// to the RaftCore notification channel, translating IOId and storage errors to notifications.
///
/// To reduce wakeup overhead, notifications are batched: at most one notification
/// is forwarded per `BATCH_INTERVAL`. When a change arrives, the forwarder waits
/// until the interval expires before reading and forwarding the latest value.
async fn io_completion_forwarder<C>(
    mut rx_io: WatchReceiverOf<C, Result<IOId<C>, StorageError<C>>>,
    weak_tx_notify: MpscWeakSenderOf<C, Notification<C>>,
) where
    C: RaftTypeConfig,
{
    const BATCH_INTERVAL: Duration = Duration::from_micros(1);

    loop {
        let deadline = C::now() + BATCH_INTERVAL;

        // Wait for IO completion notification
        if rx_io.changed().await.is_err() {
            // Watch sender dropped, exit forwarder
            tracing::debug!("IO completion watch channel closed, forwarder exiting");
            break;
        }

        let now = C::now();
        if now < deadline {
            C::sleep_until(deadline).await;

            // Drain all the changed events.
            let _ = rx_io.changed().now_or_never();
        }

        // Read the latest value after batching interval
        let result = {
            let borrowed = rx_io.borrow_watched();
            borrowed.clone()
        };

        // Try to upgrade weak sender
        let Some(tx) = weak_tx_notify.upgrade() else {
            tracing::debug!("Notification channel closed, forwarder exiting");
            break;
        };

        // Forward the result to notification channel
        let notification = match result {
            Ok(io_id) => Notification::LocalIO { io_id },
            Err(storage_error) => Notification::StorageError { error: storage_error },
        };

        if let Err(e) = tx.send(notification).await {
            tracing::warn!("failed to forward IO completion: {}", e.0);
            break;
        }
    }
}

impl<C, SM> Raft<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
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
    /// ### Recovering committed state on startup
    /// `new()` returns as soon as the node task is spawned, but the state machine may still lag the
    /// state it had before a restart — see [`RaftLogStorage::save_committed`]. An
    /// application that serves reads immediately (especially when `committed` is not persisted) may
    /// then observe a reverted state. To prevent that, await
    /// [`wait_for_recovery`](Self::wait_for_recovery) before serving reads:
    ///
    /// ```ignore
    /// let raft = Raft::new(id, config, network, log_store, sm).await?;
    /// raft.wait_for_recovery(Some(Duration::from_secs(5))).await?;
    /// // The state machine has recovered at least its pre-restart committed state.
    /// ```
    #[since(
        version = "0.10.0",
        change = "require N::Network: NetSnapshot<SnapshotData = SM::SnapshotData>"
    )]
    #[tracing::instrument(level="debug", skip_all, fields(cluster=%config.cluster_name))]
    pub async fn new<LS, N>(
        id: C::NodeId,
        config: Arc<Config>,
        network: N,
        mut log_store: LS,
        mut state_machine: SM,
    ) -> Result<Self, Fatal<C>>
    where
        N: RaftNetworkFactory<C>,
        N::Network: NetSnapshot<C, SnapshotData = SM::SnapshotData>,
        LS: RaftLogStorage<C>,
    {
        let api_channel_size = config.api_channel_size();
        let notification_channel_size = config.notification_channel_size();

        let (tx_api, rx_api) = C::mpsc(api_channel_size);
        let (tx_install_snapshot, rx_install_snapshot) = C::mpsc(api_channel_size);
        let (tx_notify, rx_notify) = C::mpsc(notification_channel_size);
        let (tx_metrics, rx_metrics) = C::watch_channel(RaftMetrics::new_initial(id.clone()));
        let (tx_data_metrics, rx_data_metrics) = C::watch_channel(RaftDataMetrics::default());
        let (tx_server_metrics, rx_server_metrics) = C::watch_channel(RaftServerMetrics::new_initial(id.clone()));

        // Watch channel for IO completion notifications from storage callbacks.
        // Initial value is a dummy IOId with this node's ID.
        let leader_id = C::LeaderId::new_with_default_term(id.clone());
        let dummy_io_id = IOId::Vote(UncommittedVote::new(leader_id));
        let (tx_io_completed, rx_io_completed) = C::watch_channel(Ok(dummy_io_id));

        // Create weak sender for forwarder before moving tx_notify into RaftCore
        let weak_tx_notify = tx_notify.downgrade();

        let (tx_progress, progress_watcher) = IoProgressWatcher::new();
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
            let mut helper = StorageHelper::new(&mut log_store, &mut state_machine).with_id(id.clone());
            helper.get_initial_state().await?
        };

        let engine = Engine::new(state, eng_config);

        let sm_span = tracing::span!(parent: &core_span, Level::DEBUG, "sm_worker");

        let sm_handle = worker::Worker::spawn(
            id.clone(),
            state_machine,
            log_store.get_log_reader().await,
            tx_notify.clone(),
            config.state_machine_channel_size(),
            sm_span,
        );

        let sm_cmd_tx = sm_handle.downgrade_sender();

        let default_io_id = IOId::new_vote_io(UncommittedVote::new_with_default_term(id.clone()));
        let (io_accepted_tx, _io_accepted_rx) = C::watch_channel(default_io_id.clone());
        let (io_submitted_tx, _io_submitted_rx) = C::watch_channel(default_io_id);
        let (committed_tx, _committed_rx) = C::watch_channel(None);

        let shared_replicate_batch = SharedReplicateBatch::new();

        let core: RaftCore<C, N, LS, SM> = RaftCore {
            id: id.clone(),
            config: config.clone(),
            runtime_config: runtime_config.clone(),
            core_state: Default::default(),
            network_factory: network,
            log_store,
            sm_handle,

            engine,

            // initially, allocate for 8 kilo outstanding requests.
            client_responders: ClientResponderQueue::with_capacity(1024 * 8),

            replications: Default::default(),

            heartbeat_handle: HeartbeatWorkersHandle::new(id.clone(), config.clone()),
            tx_api: tx_api.clone(),
            rx_api: BatchRaftMsgReceiver::new(
                rx_api,
                config.api_batch_capacity,
                Duration::from_millis(config.api_batch_linger_ms),
            ),
            tx_install_snapshot: tx_install_snapshot.clone(),
            rx_install_snapshot,

            tx_notification: tx_notify,
            rx_notification: rx_notify,

            tx_io_completed,

            io_accepted_tx,

            io_submitted_tx,

            committed_tx,
            tx_metrics,
            tx_data_metrics,
            tx_server_metrics,
            tx_progress,

            runtime_stats: RuntimeStats::new(&config),
            shared_replicate_batch,

            metrics_recorder: None,

            span: core_span,
        };

        // Spawn forwarder task to bridge Watch channel to notification channel
        let _forwarder_handle = C::spawn(io_completion_forwarder::<C>(rx_io_completed, weak_tx_notify));

        StepDownWatcher::<C>::spawn(
            rx_server_metrics.clone(),
            rx_metrics.clone(),
            tx_api.downgrade(),
            &config,
        );

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
            progress_watcher,
            tx_shutdown: Mutex::new(Some(tx_shutdown)),
            core_state: Mutex::new(CoreState::Running(core_handle)),
            extensions: Extensions::default(),
        };

        Ok(Self {
            inner: Arc::new(inner),
            sm_cmd_tx,
            install_snapshot_tx: tx_install_snapshot,
        })
    }
}

impl<C, SM> Raft<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
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
    pub fn runtime_config(&self) -> RuntimeConfigHandle<'_, C> {
        RuntimeConfigHandle::new(self.inner.as_ref())
    }

    /// Return the config of this Raft node.
    pub fn config(&self) -> &Arc<Config> {
        &self.inner.config
    }

    /// Access the underlying extensions map.
    ///
    /// For most use cases, prefer [`extension()`](Self::extension) which provides
    /// a simpler API for getting values.
    ///
    /// This method is useful when you need direct access to the [`Extensions`] type,
    /// such as checking if a value exists with [`contains()`](Extensions::contains)
    /// or removing a value with [`remove()`](Extensions::remove).
    #[since(version = "0.10.0")]
    pub fn extensions(&self) -> &Extensions {
        &self.inner.extensions
    }

    /// Get a clone of a user-defined extension value.
    ///
    /// If no value exists, `T::default()` is inserted and a clone is returned.
    /// Values must implement `Clone` and `Default`. Use `Arc` for shared mutable state.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use std::sync::atomic::{AtomicU64, Ordering};
    /// use std::sync::Arc;
    ///
    /// #[derive(Clone, Default)]
    /// pub struct MyCounter(Arc<AtomicU64>);
    ///
    /// // Get a clone (auto-inserts default if not present)
    /// let counter = raft.extension::<MyCounter>();
    /// counter.0.fetch_add(1, Ordering::Relaxed);
    ///
    /// // Multiple calls share the same underlying data via Arc
    /// let counter2 = raft.extension::<MyCounter>();
    /// assert_eq!(counter2.0.load(Ordering::Relaxed), 1);
    /// ```
    #[since(version = "0.10.0")]
    pub fn extension<T>(&self) -> T
    where T: OptionalSend + Clone + Default + 'static {
        self.inner.extensions.get::<T>()
    }

    /// Return a copy of the runtime statistics.
    ///
    /// Sends a message to RaftCore to retrieve the current runtime statistics.
    /// This returns a snapshot of the stats at the time of the call.
    #[cfg(feature = "runtime-stats")]
    pub async fn runtime_stats(&self) -> Result<RuntimeStats<C>, Fatal<C>> {
        self.inner.call_core_oneshot(|tx| RaftMsg::GetRuntimeStats { tx }).await
    }

    /// Check if this node is currently the leader.
    ///
    /// Returns `true` if the node's current state is [`ServerState::Leader`].
    ///
    /// # Example
    ///
    /// ```ignore
    /// if raft.is_leader() {
    ///     // Perform leader-only operations
    /// }
    /// ```
    ///
    /// [`ServerState::Leader`]: crate::core::ServerState::Leader
    #[since(version = "0.10.0")]
    pub fn is_leader(&self) -> bool {
        self.inner.rx_metrics.borrow_watched().state.is_leader()
    }

    /// Get leader information if this node is currently a leader.
    ///
    /// Returns [`Leader`] containing the leader ID and health metadata if this node is the leader
    /// (i.e., its vote has been accepted by a quorum), otherwise returns
    /// [`ForwardToLeader`] error containing the current known leader information.
    ///
    /// # Example
    ///
    /// ```ignore
    /// match raft.as_leader() {
    ///     Ok(leader) => {
    ///         println!("This node is leader: {:?}", leader.leader_id());
    ///     }
    ///     Err(forward) => {
    ///         println!("Forward to leader: {:?}", forward.leader_id);
    ///     }
    /// }
    /// ```
    ///
    /// [`ForwardToLeader`]: crate::errors::ForwardToLeader
    #[since(version = "0.10.0")]
    pub fn as_leader(&self) -> Result<Leader<C, SM>, ForwardToLeader<C>> {
        // Do not use `is_leader()`, which depends on other state to determine, which may result in
        // inconsistent state. And `is_leader()` just do another reading from the metrics, which also may be
        // inconsistent.

        let metrics = self.inner.rx_metrics.borrow_watched();

        let Some(committed_vote) = metrics.vote.try_to_committed() else {
            return Err(ForwardToLeader::empty());
        };

        let leader_id = committed_vote.leader_id();
        let node_id = leader_id.node_id();

        if node_id == &self.inner.id {
            Ok(Leader {
                raft: self.clone(),
                leader_id: leader_id.clone(),
                last_quorum_acked: metrics.last_quorum_acked.map(|s| s.into_inner()),
            })
        } else {
            let node = metrics.membership_config.membership().get_node(node_id).cloned();

            Err(ForwardToLeader {
                leader_id: Some(node_id.clone()),
                leader_node: node,
            })
        }
    }

    /// Get the ID of this Raft node.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let id = raft.node_id();
    /// println!("Node ID: {:?}", id);
    /// ```
    #[since(version = "0.10.0")]
    pub fn node_id(&self) -> &C::NodeId {
        &self.inner.id
    }

    /// Get an iterator over the current voter node IDs.
    ///
    /// Returns node IDs that are voters in the effective membership. Learners are not included.
    ///
    /// # Example
    ///
    /// ```ignore
    /// for voter_id in raft.voter_ids() {
    ///     println!("Voter: {:?}", voter_id);
    /// }
    /// ```
    #[since(version = "0.10.0")]
    pub fn voter_ids(&self) -> impl Iterator<Item = C::NodeId> {
        // borrow_watched() holds a lock that blocks RaftCore.
        // Clone and collect immediately to release the lock quickly.
        let membership = self.inner.rx_metrics.borrow_watched().membership_config.clone();
        membership.voter_ids().collect::<Vec<_>>().into_iter()
    }

    /// Get an iterator over the current learner node IDs.
    ///
    /// Returns node IDs that are learners in the effective membership. Voters are not included.
    ///
    /// # Example
    ///
    /// ```ignore
    /// for learner_id in raft.learner_ids() {
    ///     println!("Learner: {:?}", learner_id);
    /// }
    /// ```
    #[since(version = "0.10.0")]
    pub fn learner_ids(&self) -> impl Iterator<Item = C::NodeId> {
        // borrow_watched() holds a lock that blocks RaftCore.
        // Clone and collect immediately to release the lock quickly.
        let membership = self.inner.rx_metrics.borrow_watched().membership_config.clone();
        membership.membership().learner_ids().collect::<Vec<_>>().into_iter()
    }

    /// Create a new [`ProtocolApi`] to handle Raft protocol RPCs received by this Raft node.
    ///
    /// [`ProtocolApi`] provides the following protocol APIs:
    /// - [`ProtocolApi::append_entries`]
    /// - [`ProtocolApi::vote`]
    /// - [`ProtocolApi::get_snapshot`]
    /// - [`ProtocolApi::begin_receiving_snapshot`]
    /// - [`ProtocolApi::install_full_snapshot`]
    /// - [`ProtocolApi::handle_transfer_leader`]
    pub(crate) fn protocol_api(&self) -> ProtocolApi<C, SM> {
        ProtocolApi::new(
            self.inner.clone(),
            self.sm_cmd_tx.clone(),
            self.install_snapshot_tx.clone(),
        )
    }

    pub(crate) fn app_api(&self) -> AppApi<'_, C> {
        AppApi::new(&self.inner)
    }

    pub(crate) fn management_api(&self) -> ManagementApi<'_, C> {
        ManagementApi::new(self.inner.as_ref())
    }

    /// Return a [`Trigger`] handle to manually trigger raft actions, such as elect or build
    /// snapshot.
    ///
    /// Example:
    /// ```ignore
    /// let raft = Raft::new(...).await?;
    /// raft.trigger().elect(false).await?;
    /// ```
    pub fn trigger(&self) -> Trigger<'_, C> {
        Trigger::new(self.inner.as_ref())
    }

    /// Set or unset a custom metrics recorder for exporting Raft metrics.
    ///
    /// This allows applications to plug in their own metrics collection backends
    /// (e.g., OpenTelemetry, Prometheus, StatsD) at runtime. The recorder will
    /// receive metrics events from RaftCore as they occur.
    ///
    /// Pass `Some(recorder)` to enable metrics recording, or `None` to disable it.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use std::sync::Arc;
    /// use openraft::metrics::MetricsRecorder;
    ///
    /// struct MyRecorder;
    /// impl MetricsRecorder for MyRecorder {
    ///     fn record_apply_batch(&self, entry_count: u64) { /* ... */ }
    ///     fn record_append_batch(&self, entry_count: u64) { /* ... */ }
    /// }
    ///
    /// // Enable metrics recording
    /// raft.set_metrics_recorder(Some(Arc::new(MyRecorder))).await?;
    ///
    /// // Disable metrics recording
    /// raft.set_metrics_recorder(None).await?;
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Fatal`] error if RaftCore is shut down or has a storage error.
    pub async fn set_metrics_recorder(&self, recorder: Option<Arc<dyn MetricsRecorder>>) -> Result<(), Fatal<C>> {
        self.inner.send_external_command(ExternalCommand::SetMetricsRecorder { recorder }).await
    }
}
