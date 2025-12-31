//! Defines the [`NetTransferLeader`] trait for leader transfer.

use openraft_macros::add_async_trait;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::error::RPCError;
use crate::network::RPCOption;
use crate::raft::message::TransferLeaderRequest;

/// Sends TransferLeader messages to a target node.
///
/// This trait is part of the granular network API. It can be implemented directly
/// or automatically via blanket impl when implementing [`RaftNetworkV2`].
///
/// [`RaftNetworkV2`]: crate::network::RaftNetworkV2
#[add_async_trait]
pub trait NetTransferLeader<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send TransferLeader message to the target node.
    ///
    /// The node received this message should pass it to [`Raft::handle_transfer_leader()`].
    ///
    /// [`Raft::handle_transfer_leader()`]: crate::raft::Raft::handle_transfer_leader
    async fn transfer_leader(&mut self, req: TransferLeaderRequest<C>, option: RPCOption) -> Result<(), RPCError<C>>;
}
