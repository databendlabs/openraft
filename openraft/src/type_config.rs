//! Define the configuration of types used by the Raft, such as [`NodeId`], log [`Entry`], etc.
//!
//! [`NodeId`]: `RaftTypeConfig::NodeId`
//! [`Entry`]: `RaftTypeConfig::Entry`

pub(crate) mod util;

use std::fmt::Debug;

pub use util::TypeConfigExt;

use crate::entry::FromAppData;
use crate::entry::RaftEntry;
use crate::raft::responder::Responder;
use crate::AppData;
use crate::AppDataResponse;
use crate::AsyncRuntime;
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
///        Node         = openraft::BasicNode,
///        Entry        = openraft::Entry<TypeConfig>,
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

    /// Raft log entry, which can be built from an AppData.
    type Entry: RaftEntry<Self> + FromAppData<Self::D>;

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
    use crate::raft::responder::Responder;
    use crate::AsyncRuntime;
    use crate::RaftTypeConfig;

    pub type DOf<C> = <C as RaftTypeConfig>::D;
    pub type ROf<C> = <C as RaftTypeConfig>::R;
    pub type NodeIdOf<C> = <C as RaftTypeConfig>::NodeId;
    pub type NodeOf<C> = <C as RaftTypeConfig>::Node;
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
    pub type OneshotSenderOf<C, T> = <Rt<C> as AsyncRuntime>::OneshotSender<T>;
    pub type OneshotReceiverErrorOf<C> = <Rt<C> as AsyncRuntime>::OneshotReceiverError;
    pub type OneshotReceiverOf<C, T> = <Rt<C> as AsyncRuntime>::OneshotReceiver<T>;

    // Usually used types
    pub type LogIdOf<C> = crate::LogId<NodeIdOf<C>>;
    pub type VoteOf<C> = crate::Vote<NodeIdOf<C>>;
    pub type LeaderIdOf<C> = crate::LeaderId<NodeIdOf<C>>;
    pub type CommittedLeaderIdOf<C> = crate::CommittedLeaderId<NodeIdOf<C>>;
}
