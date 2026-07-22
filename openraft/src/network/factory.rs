use openraft_macros::add_async_trait;
use openraft_macros::since;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::network::NetBackoff;
use crate::network::NetSnapshot;
use crate::network::NetStreamAppend;
use crate::network::NetTransferLeader;
use crate::network::NetVote;

/// A trait defining the interface for a Raft network factory to create connections between cluster
/// members.
///
/// See the [network chapter of the guide](crate::docs::getting_started#4-implement-raftnetwork)
/// for details and discussion on this trait and how to implement it.
///
/// ## Related Types
///
/// - [`RaftLogStorage`](crate::storage::RaftLogStorage) - For log storage operations
/// - [`RaftStateMachine`](crate::storage::RaftStateMachine) - For state machine operations
/// - [`Config`](crate::config::Config) - For configuration options
///
/// Typically, the network implementation as such will be hidden behind a `Box<T>` or `Arc<T>` and
/// this interface implemented on the `Box<T>` or `Arc<T>`.
#[add_async_trait]
pub trait RaftNetworkFactory<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Actual type of the network handling a single connection.
    type Network: NetBackoff<C> + NetStreamAppend<C> + NetVote<C> + NetSnapshot<C> + NetTransferLeader<C>;

    /// Create a new network instance sending RPCs to the target node.
    ///
    /// This function should **not** create a connection but rather a client that will connect when
    /// required. Therefore, there is a chance it will build a client that is unable to send out
    /// anything, e.g., in case the Node network address is configured incorrectly. But this method
    /// does not return an error because openraft can only ignore it.
    ///
    /// The method is intentionally async to give the implementation a chance to use asynchronous
    /// sync primitives to serialize access to the common internal object, if needed.
    async fn new_client(&mut self, target: C::NodeId, node: &C::Node) -> Self::Network;

    /// Create a network client dedicated to sending heartbeat RPCs to the target node.
    ///
    /// This client carries the Leader's liveness probes: the periodic heartbeat that maintains the
    /// Leader lease and the lease-confirmation probe issued for linearizable reads. These must not
    /// be delayed by replication backpressure. On a transport that cannot multiplex a single
    /// connection (e.g., HTTP/1.1), sharing the replication client lets a large `AppendEntries`
    /// batch or a snapshot transfer stall heartbeats through head-of-line blocking. Override this
    /// method to isolate these probes on an independent connection when that matters.
    ///
    /// The default implementation delegates to [`new_client()`](Self::new_client), so heartbeats
    /// share the replication path unless overridden.
    #[since(version = "0.10.0")]
    async fn new_heartbeat_client(&mut self, target: C::NodeId, node: &C::Node) -> Self::Network {
        self.new_client(target, node).await
    }

    /// Create a network client dedicated to transmitting snapshots to the target node.
    ///
    /// Snapshot transfers can be large and long-running. On a transport that cannot multiplex a
    /// single connection, sharing the replication client lets an in-flight snapshot block ordinary
    /// log replication. Override this method to isolate snapshot transfer on its own connection.
    ///
    /// The default implementation delegates to [`new_client()`](Self::new_client), so snapshot
    /// transfer shares the replication path unless overridden.
    #[since(version = "0.10.0")]
    async fn new_snapshot_client(&mut self, target: C::NodeId, node: &C::Node) -> Self::Network {
        self.new_client(target, node).await
    }
}
