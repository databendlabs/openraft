use macros::add_async_trait;

use crate::network::RaftNetwork;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;

/// A trait defining the interface for a Raft network factory to create connections between cluster
/// members.
///
/// See the [network chapter of the guide](https://datafuselabs.github.io/openraft/getting-started.html#3-impl-raftnetwork)
/// for details and discussion on this trait and how to implement it.
///
/// Typically, the network implementation as such will be hidden behind a `Box<T>` or `Arc<T>` and
/// this interface implemented on the `Box<T>` or `Arc<T>`.
#[add_async_trait]
pub trait RaftNetworkFactory<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Actual type of the network handling a single connection.
    type Network: RaftNetwork<C>;

    /// Create a new network instance sending RPCs to the target node.
    ///
    /// This function should **not** create a connection but rather a client that will connect when
    /// required. Therefore there is chance it will build a client that is unable to send out
    /// anything, e.g., in case the Node network address is configured incorrectly. But this method
    /// does not return an error because openraft can only ignore it.
    ///
    /// The method is intentionally async to give the implementation a chance to use asynchronous
    /// sync primitives to serialize access to the common internal object, if needed.
    async fn new_client(&mut self, target: C::NodeId, node: &C::Node) -> Self::Network;
}
