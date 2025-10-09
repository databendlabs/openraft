//! Define the configuration of types used by the Raft, such as [`NodeId`], log [`Entry`], etc.
//!
//! [`NodeId`]: `RaftTypeConfig::NodeId`
//! [`Entry`]: `RaftTypeConfig::Entry`

pub mod async_runtime;
pub(crate) mod util;

use std::fmt::Debug;

pub use async_runtime::AsyncRuntime;
pub use async_runtime::MpscUnbounded;
pub use async_runtime::OneshotSender;
pub use util::TypeConfigExt;

use crate::AppData;
use crate::AppDataResponse;
use crate::Node;
use crate::NodeId;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::entry::RaftEntry;
use crate::raft::responder::Responder;
use crate::vote::RaftLeaderId;
use crate::vote::RaftTerm;
use crate::vote::raft_vote::RaftVote;

/// Type configuration for customizing Raft components.
///
/// `RaftTypeConfig` defines all pluggable types used by Raft, including application data types,
/// node identifiers, async runtime, and internal data structures. Applications implement this
/// trait (typically via the [`declare_raft_types!`] macro) to configure Raft for their needs.
///
/// # Examples
///
/// ## Minimal Configuration
///
/// Use defaults for all types except application data:
///
/// ```ignore
/// openraft::declare_raft_types!(
///     pub MyTypeConfig:
///         D = String,
///         R = String,
/// );
/// ```
///
/// ## Full Configuration
///
/// Specify all types explicitly:
///
/// ```ignore
/// openraft::declare_raft_types!(
///     pub MyTypeConfig:
///         D            = ClientRequest,
///         R            = ClientResponse,
///         NodeId       = u64,
///         Node         = openraft::impls::BasicNode,
///         Term         = u64,
///         LeaderId     = openraft::impls::leader_id_adv::LeaderId<Self>,
///         Vote         = openraft::impls::Vote<Self>,
///         Entry        = openraft::impls::Entry<Self>,
///         SnapshotData = Cursor<Vec<u8>>,
///         Responder    = openraft::impls::OneshotResponder<Self>,
///         AsyncRuntime = openraft::impls::TokioRuntime,
/// );
/// ```
///
/// Then use your config with Raft:
///
/// ```ignore
/// type MyRaft = Raft<MyTypeConfig>;
/// let raft = MyRaft::new(id, config, network, log_store, state_machine).await?;
/// ```
///
/// # See Also
///
/// - [`declare_raft_types!`] macro for easy type configuration
/// - Each associated type's documentation for requirements and defaults
///
/// [`Raft`]: crate::Raft
/// [`declare_raft_types!`]: crate::declare_raft_types
pub trait RaftTypeConfig:
    Sized + OptionalSend + OptionalSync + Debug + Clone + Copy + Default + Eq + PartialEq + Ord + PartialOrd + 'static
{
    /// Application-specific request data passed to the state machine.
    type D: AppData;

    /// Application-specific response data returned by the state machine.
    type R: AppDataResponse;

    /// A Raft node's ID.
    type NodeId: NodeId;

    /// Raft application level node data
    type Node: Node;

    /// Type representing a Raft term number.
    ///
    /// A term is a logical clock in Raft that is used to detect obsolete information,
    /// such as old leaders. It must be totally ordered and monotonically increasing.
    ///
    /// Common implementations are provided for standard integer types like `u64`, `i64`, etc.
    ///
    /// See: [`RaftTerm`] for the required methods.
    type Term: RaftTerm;

    /// A Leader identifier in a cluster.
    type LeaderId: RaftLeaderId<Self>;

    /// Raft vote type.
    ///
    /// It represents a candidate's vote or a leader's vote that has been granted by a quorum.
    type Vote: RaftVote<Self>;

    /// Raft log entry, which can be built from an AppData.
    type Entry: RaftEntry<Self>;

    /// Snapshot data for exposing a snapshot for reading & writing.
    ///
    /// See the [storage chapter of the guide][sto] for details on log compaction / snapshotting.
    ///
    /// [sto]: crate::docs::getting_started#3-implement-raftlogstorage-and-raftstatemachine
    type SnapshotData: OptionalSend + 'static;

    /// Asynchronous runtime type.
    type AsyncRuntime: AsyncRuntime;

    /// Send the response or error of a client write request([`WriteResult`]).
    ///
    /// For example, return [`WriteResult`] to the caller of [`Raft::client_write`], or send to
    /// some application-defined channel.
    ///
    /// [`Raft::client_write`]: `crate::raft::Raft::client_write`
    /// [`WriteResult`]: `crate::raft::message::ClientWriteResult`
    type Responder: Responder<Self>;
}

#[allow(dead_code)]
/// Type alias for types used in `RaftTypeConfig`.
///
/// Aliases are enabled by the feature flag [`type-alias`].
///
/// [`type-alias`]: crate::docs::feature_flags#feature-flag-type-alias
pub mod alias {
    use crate::EntryPayload;
    use crate::LogId;
    use crate::RaftTypeConfig;
    use crate::async_runtime::Mpsc;
    use crate::async_runtime::MpscUnbounded;
    use crate::async_runtime::Oneshot;
    use crate::async_runtime::watch;
    use crate::raft::responder::Responder;
    use crate::type_config::AsyncRuntime;
    use crate::vote::RaftLeaderId;

    /// Type alias for application data.
    pub type DOf<C> = <C as RaftTypeConfig>::D;
    /// Type alias for application response.
    pub type ROf<C> = <C as RaftTypeConfig>::R;
    /// Type alias for application data (alternative name).
    pub type AppDataOf<C> = <C as RaftTypeConfig>::D;
    /// Type alias for application response (alternative name).
    pub type AppResponseOf<C> = <C as RaftTypeConfig>::R;
    /// Type alias for node ID.
    pub type NodeIdOf<C> = <C as RaftTypeConfig>::NodeId;
    /// Type alias for node.
    pub type NodeOf<C> = <C as RaftTypeConfig>::Node;
    /// Type alias for term.
    pub type TermOf<C> = <C as RaftTypeConfig>::Term;
    /// Type alias for leader ID.
    pub type LeaderIdOf<C> = <C as RaftTypeConfig>::LeaderId;
    /// Type alias for vote.
    pub type VoteOf<C> = <C as RaftTypeConfig>::Vote;
    /// Type alias for log entry.
    pub type EntryOf<C> = <C as RaftTypeConfig>::Entry;
    /// Type alias for snapshot data.
    pub type SnapshotDataOf<C> = <C as RaftTypeConfig>::SnapshotData;
    /// Type alias for async runtime.
    pub type AsyncRuntimeOf<C> = <C as RaftTypeConfig>::AsyncRuntime;
    /// Type alias for responder.
    pub type ResponderOf<C> = <C as RaftTypeConfig>::Responder;
    /// Type alias for responder receiver.
    pub type ResponderReceiverOf<C> = <ResponderOf<C> as Responder<C>>::Receiver;

    type Rt<C> = AsyncRuntimeOf<C>;

    /// Type alias for async runtime join error.
    pub type JoinErrorOf<C> = <Rt<C> as AsyncRuntime>::JoinError;
    /// Type alias for async runtime join handle.
    pub type JoinHandleOf<C, T> = <Rt<C> as AsyncRuntime>::JoinHandle<T>;
    /// Type alias for async runtime sleep future.
    pub type SleepOf<C> = <Rt<C> as AsyncRuntime>::Sleep;
    /// Type alias for async runtime instant.
    pub type InstantOf<C> = <Rt<C> as AsyncRuntime>::Instant;
    /// Type alias for async runtime timeout error.
    pub type TimeoutErrorOf<C> = <Rt<C> as AsyncRuntime>::TimeoutError;
    /// Type alias for async runtime timeout future.
    pub type TimeoutOf<C, R, F> = <Rt<C> as AsyncRuntime>::Timeout<R, F>;

    /// Type alias for oneshot channel implementation.
    pub type OneshotOf<C> = <Rt<C> as AsyncRuntime>::Oneshot;
    /// Type alias for oneshot channel sender.
    pub type OneshotSenderOf<C, T> = <OneshotOf<C> as Oneshot>::Sender<T>;
    /// Type alias for oneshot channel receiver error.
    pub type OneshotReceiverErrorOf<C> = <OneshotOf<C> as Oneshot>::ReceiverError;
    /// Type alias for oneshot channel receiver.
    pub type OneshotReceiverOf<C, T> = <OneshotOf<C> as Oneshot>::Receiver<T>;

    /// Type alias for bounded MPSC channel implementation.
    pub type MpscOf<C> = <Rt<C> as AsyncRuntime>::Mpsc;

    // MPSC bounded
    type MpscB<C> = MpscOf<C>;

    /// Type alias for bounded MPSC channel sender.
    pub type MpscSenderOf<C, T> = <MpscB<C> as Mpsc>::Sender<T>;
    /// Type alias for bounded MPSC channel receiver.
    pub type MpscReceiverOf<C, T> = <MpscB<C> as Mpsc>::Receiver<T>;
    /// Type alias for bounded MPSC channel weak sender.
    pub type MpscWeakSenderOf<C, T> = <MpscB<C> as Mpsc>::WeakSender<T>;

    /// Type alias for unbounded MPSC channel implementation.
    pub type MpscUnboundedOf<C> = <Rt<C> as AsyncRuntime>::MpscUnbounded;

    // MPSC unbounded
    type MpscUB<C> = MpscUnboundedOf<C>;

    /// Type alias for unbounded MPSC channel sender.
    pub type MpscUnboundedSenderOf<C, T> = <MpscUB<C> as MpscUnbounded>::Sender<T>;
    /// Type alias for unbounded MPSC channel receiver.
    pub type MpscUnboundedReceiverOf<C, T> = <MpscUB<C> as MpscUnbounded>::Receiver<T>;
    /// Type alias for unbounded MPSC channel weak sender.
    pub type MpscUnboundedWeakSenderOf<C, T> = <MpscUB<C> as MpscUnbounded>::WeakSender<T>;

    /// Type alias for watch channel implementation.
    pub type WatchOf<C> = <Rt<C> as AsyncRuntime>::Watch;
    /// Type alias for watch channel sender.
    pub type WatchSenderOf<C, T> = <WatchOf<C> as watch::Watch>::Sender<T>;
    /// Type alias for watch channel receiver.
    pub type WatchReceiverOf<C, T> = <WatchOf<C> as watch::Watch>::Receiver<T>;

    /// Type alias for async mutex.
    pub type MutexOf<C, T> = <Rt<C> as AsyncRuntime>::Mutex<T>;

    // Usually used types
    /// Type alias for log ID.
    pub type LogIdOf<C> = LogId<C>;
    /// Type alias for committed leader ID.
    pub type CommittedLeaderIdOf<C> = <LeaderIdOf<C> as RaftLeaderId<C>>::Committed;
    /// Type alias for entry payload.
    pub type EntryPayloadOf<C> = EntryPayload<C>;
    /// Type alias for serializable instant.
    pub type SerdeInstantOf<C> = crate::metrics::SerdeInstant<InstantOf<C>>;
}
