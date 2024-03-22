use std::fmt::Debug;

use crate::entry::FromAppData;
use crate::entry::RaftEntry;
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
    #[cfg(not(feature = "generic-snapshot-data"))]
    type SnapshotData: tokio::io::AsyncRead
        + tokio::io::AsyncWrite
        + tokio::io::AsyncSeek
        + OptionalSend
        + Unpin
        + 'static;
    #[cfg(feature = "generic-snapshot-data")]
    type SnapshotData: OptionalSend + 'static;

    /// Asynchronous runtime type.
    type AsyncRuntime: AsyncRuntime;
}

#[allow(dead_code)]
pub mod alias {
    //! Type alias for types used in `RaftTypeConfig`.

    pub type DOf<C> = <C as crate::RaftTypeConfig>::D;
    pub type ROf<C> = <C as crate::RaftTypeConfig>::R;
    pub type NodeIdOf<C> = <C as crate::RaftTypeConfig>::NodeId;
    pub type NodeOf<C> = <C as crate::RaftTypeConfig>::Node;
    pub type EntryOf<C> = <C as crate::RaftTypeConfig>::Entry;
    pub type SnapshotDataOf<C> = <C as crate::RaftTypeConfig>::SnapshotData;
    pub type AsyncRuntimeOf<C> = <C as crate::RaftTypeConfig>::AsyncRuntime;

    pub type JoinErrorOf<C> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::JoinError;
    pub type JoinHandleOf<C, T> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::JoinHandle<T>;
    pub type SleepOf<C> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::Sleep;
    pub type InstantOf<C> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::Instant;
    pub type TimeoutErrorOf<C> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::TimeoutError;
    pub type TimeoutOf<C, R, F> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::Timeout<R, F>;
    pub type OneshotSenderOf<C, T> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::OneshotSender<T>;
    pub type OneshotReceiverErrorOf<C> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::OneshotReceiverError;
    pub type OneshotReceiverOf<C, T> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::OneshotReceiver<T>;

    // Usually used types
    pub type LogIdOf<C> = crate::LogId<NodeIdOf<C>>;
    pub type VoteOf<C> = crate::Vote<NodeIdOf<C>>;
    pub type LeaderIdOf<C> = crate::LeaderId<NodeIdOf<C>>;
    pub type CommittedLeaderIdOf<C> = crate::CommittedLeaderId<NodeIdOf<C>>;
}
