use std::fmt::Debug;

use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::io::AsyncWrite;

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
    type Entry: RaftEntry<Self::NodeId, Self::Node> + FromAppData<Self::D>;

    /// Snapshot data for exposing a snapshot for reading & writing.
    ///
    /// See the [storage chapter of the guide](https://datafuselabs.github.io/openraft/getting-started.html#implement-raftstorage)
    /// for details on where and how this is used.
    type SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + OptionalSend + OptionalSync + Unpin + 'static;

    /// Asynchronous runtime type.
    type AsyncRuntime: AsyncRuntime;
}

#[allow(dead_code)]
pub(crate) mod alias {
    //! Type alias for types used in `RaftTypeConfig`.

    pub(crate) type DOf<C> = <C as crate::RaftTypeConfig>::D;
    pub(crate) type ROf<C> = <C as crate::RaftTypeConfig>::R;
    pub(crate) type NodeIdOf<C> = <C as crate::RaftTypeConfig>::NodeId;
    pub(crate) type NodeOf<C> = <C as crate::RaftTypeConfig>::Node;
    pub(crate) type EntryOf<C> = <C as crate::RaftTypeConfig>::Entry;
    pub(crate) type SnapshotDataOf<C> = <C as crate::RaftTypeConfig>::SnapshotData;
    pub(crate) type AsyncRuntimeOf<C> = <C as crate::RaftTypeConfig>::AsyncRuntime;

    pub(crate) type JoinErrorOf<C> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::JoinError;
    pub(crate) type JoinHandleOf<C, T> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::JoinHandle<T>;
    pub(crate) type SleepOf<C> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::Sleep;
    pub(crate) type InstantOf<C> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::Instant;
    pub(crate) type TimeoutErrorOf<C> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::TimeoutError;
    pub(crate) type TimeoutOf<C, R, F> = <AsyncRuntimeOf<C> as crate::AsyncRuntime>::Timeout<R, F>;

    // Usually used types
    pub(crate) type LogIdOf<C> = crate::LogId<NodeIdOf<C>>;
    pub(crate) type VoteOf<C> = crate::Vote<NodeIdOf<C>>;
    pub(crate) type LeaderIdOf<C> = crate::LeaderId<NodeIdOf<C>>;
    pub(crate) type CommittedLeaderIdOf<C> = crate::CommittedLeaderId<NodeIdOf<C>>;
}
