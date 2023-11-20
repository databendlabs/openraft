use std::fmt::Debug;

use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::io::AsyncWrite;

use crate::entry::FromAppData;
use crate::entry::RaftEntry;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::AppData;
use crate::AppDataResponse;
use crate::AsyncRuntime;
use crate::Node;
use crate::NodeId;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftNetworkFactory;

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
    Sized + Send + Sync + Debug + Clone + Copy + Default + Eq + PartialEq + Ord + PartialOrd + 'static
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

/// Configuration of types used by the [`Raft`] core engine for the storage.
///
/// The (empty) implementation of this structure defines network factory, log storage and
/// state machine types. Refer to the documentation of associated types for more information.
///
/// [`Raft`]: crate::Raft
// : Sized + Send + Sync + Debug + Clone + Copy + Default + Eq + PartialEq + Ord + PartialOrd +
// 'static
pub trait StorageTypeConfig<C: RaftTypeConfig> {
    /// Network factory to use to create new connections.
    type NetworkFactory: RaftNetworkFactory<C>;

    /// Log storage storing the deltas.
    type LogStorage: RaftLogStorage<C>;

    /// State machine processing requests and storing the snapshot of the data.
    type StateMachine: RaftStateMachine<C>;
}
