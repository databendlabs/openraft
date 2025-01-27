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

use crate::entry::RaftEntry;
use crate::raft::responder::Responder;
use crate::vote::raft_vote::RaftVote;
use crate::vote::RaftLeaderId;
use crate::vote::RaftTerm;
use crate::AppData;
use crate::AppDataResponse;
use crate::Node;
use crate::NodeId;
use crate::OptionalSend;
use crate::OptionalSync;

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
/// `declare_raft_types` is provided, which can be used to declare the type easily.
///
/// Example:
/// ```ignore
/// openraft::declare_raft_types!(
///    pub TypeConfig:
///        D            = ClientRequest,
///        R            = ClientResponse,
///        NodeId       = u64,
///        Node         = openraft::impls::BasicNode,
///        Term         = u64,
///        LeaderId     = openraft::impls::leader_id_adv::LeaderId<Self>,
///        Vote         = openraft::impls::Vote<Self>,
///        Entry        = openraft::impls::Entry<Self>,
///        SnapshotData = Cursor<Vec<u8>>,
///        AsyncRuntime = openraft::TokioRuntime,
/// );
/// ```
/// [`Raft`]: crate::Raft
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
    /// Common implementations are provided for standard integer types like `u64`, `i64` etc.
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
    /// For example, return [`WriteResult`] the to the caller of [`Raft::client_write`], or send to
    /// some application defined channel.
    ///
    /// [`Raft::client_write`]: `crate::raft::Raft::client_write`
    /// [`WriteResult`]: `crate::raft::message::ClientWriteResult`
    type Responder: Responder<Self>;
}

#[allow(dead_code)]
/// Type alias for types used in `RaftTypeConfig`.
///
/// Alias are enabled by feature flag [`type-alias`].
///
/// [`type-alias`]: crate::docs::feature_flags#feature-flag-type-alias
pub mod alias {
    use crate::async_runtime::watch;
    use crate::async_runtime::Mpsc;
    use crate::async_runtime::MpscUnbounded;
    use crate::async_runtime::Oneshot;
    use crate::raft::responder::Responder;
    use crate::type_config::AsyncRuntime;
    use crate::vote::RaftLeaderId;
    use crate::EntryPayload;
    use crate::LogId;
    use crate::RaftTypeConfig;

    pub type DOf<C> = <C as RaftTypeConfig>::D;
    pub type ROf<C> = <C as RaftTypeConfig>::R;
    pub type AppDataOf<C> = <C as RaftTypeConfig>::D;
    pub type AppResponseOf<C> = <C as RaftTypeConfig>::R;
    pub type NodeIdOf<C> = <C as RaftTypeConfig>::NodeId;
    pub type NodeOf<C> = <C as RaftTypeConfig>::Node;
    pub type TermOf<C> = <C as RaftTypeConfig>::Term;
    pub type LeaderIdOf<C> = <C as RaftTypeConfig>::LeaderId;
    pub type VoteOf<C> = <C as RaftTypeConfig>::Vote;
    pub type EntryOf<C> = <C as RaftTypeConfig>::Entry;
    pub type SnapshotDataOf<C> = <C as RaftTypeConfig>::SnapshotData;
    pub type AsyncRuntimeOf<C> = <C as RaftTypeConfig>::AsyncRuntime;
    pub type ResponderOf<C> = <C as RaftTypeConfig>::Responder;
    pub type ResponderReceiverOf<C> = <ResponderOf<C> as Responder<C>>::Receiver;

    type Rt<C> = AsyncRuntimeOf<C>;

    pub type JoinErrorOf<C> = <Rt<C> as AsyncRuntime>::JoinError;
    pub type JoinHandleOf<C, T> = <Rt<C> as AsyncRuntime>::JoinHandle<T>;
    pub type SleepOf<C> = <Rt<C> as AsyncRuntime>::Sleep;
    pub type InstantOf<C> = <Rt<C> as AsyncRuntime>::Instant;
    pub type TimeoutErrorOf<C> = <Rt<C> as AsyncRuntime>::TimeoutError;
    pub type TimeoutOf<C, R, F> = <Rt<C> as AsyncRuntime>::Timeout<R, F>;

    pub type OneshotOf<C> = <Rt<C> as AsyncRuntime>::Oneshot;
    pub type OneshotSenderOf<C, T> = <OneshotOf<C> as Oneshot>::Sender<T>;
    pub type OneshotReceiverErrorOf<C> = <OneshotOf<C> as Oneshot>::ReceiverError;
    pub type OneshotReceiverOf<C, T> = <OneshotOf<C> as Oneshot>::Receiver<T>;

    pub type MpscOf<C> = <Rt<C> as AsyncRuntime>::Mpsc;

    // MPSC bounded
    type MpscB<C> = MpscOf<C>;

    pub type MpscSenderOf<C, T> = <MpscB<C> as Mpsc>::Sender<T>;
    pub type MpscReceiverOf<C, T> = <MpscB<C> as Mpsc>::Receiver<T>;
    pub type MpscWeakSenderOf<C, T> = <MpscB<C> as Mpsc>::WeakSender<T>;

    pub type MpscUnboundedOf<C> = <Rt<C> as AsyncRuntime>::MpscUnbounded;

    // MPSC unbounded
    type MpscUB<C> = MpscUnboundedOf<C>;

    pub type MpscUnboundedSenderOf<C, T> = <MpscUB<C> as MpscUnbounded>::Sender<T>;
    pub type MpscUnboundedReceiverOf<C, T> = <MpscUB<C> as MpscUnbounded>::Receiver<T>;
    pub type MpscUnboundedWeakSenderOf<C, T> = <MpscUB<C> as MpscUnbounded>::WeakSender<T>;

    pub type WatchOf<C> = <Rt<C> as AsyncRuntime>::Watch;
    pub type WatchSenderOf<C, T> = <WatchOf<C> as watch::Watch>::Sender<T>;
    pub type WatchReceiverOf<C, T> = <WatchOf<C> as watch::Watch>::Receiver<T>;

    pub type MutexOf<C, T> = <Rt<C> as AsyncRuntime>::Mutex<T>;

    // Usually used types
    pub type LogIdOf<C> = LogId<C>;
    pub type CommittedLeaderIdOf<C> = <LeaderIdOf<C> as RaftLeaderId<C>>::Committed;
    pub type EntryPayloadOf<C> = EntryPayload<C>;
    pub type SerdeInstantOf<C> = crate::metrics::SerdeInstant<InstantOf<C>>;
}
