use openraft_macros::add_async_trait;

use crate::network::v2::RaftNetworkV2;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftNetworkFactory;
use crate::RaftTypeConfig;

/// A trait defining the interface for a Raft network factory to create connections between cluster
/// members.
///
/// See the [network chapter of the guide](crate::docs::getting_started#4-implement-raftnetwork)
/// for details and discussion on this trait and how to implement it.
///
/// Typically, the network implementation as such will be hidden behind a `Box<T>` or `Arc<T>` and
/// this interface implemented on the `Box<T>` or `Arc<T>`.
///
/// V2 network API removes `install_snapshot()` method that sends snapshot in chunks
/// and introduces `full_snapshot()` method that let application fully customize snapshot transmit.
///
/// Compatibility: [`RaftNetworkFactoryV2`] is automatically implemented for [`RaftNetworkFactory`]
/// implementations.
#[add_async_trait]
pub trait RaftNetworkFactoryV2<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Actual type of the network handling a single connection.
    type Network: RaftNetworkV2<C>;

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

impl<C, V1> RaftNetworkFactoryV2<C> for V1
where
    C: RaftTypeConfig,
    V1: RaftNetworkFactory<C>,
    C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin,
{
    type Network = V1::Network;

    async fn new_client(&mut self, target: C::NodeId, node: &C::Node) -> Self::Network {
        RaftNetworkFactory::<C>::new_client(self, target, node).await
    }
}
