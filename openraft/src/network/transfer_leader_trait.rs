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
/// **For most applications, implement [`RaftNetworkV2`] instead.** This trait is
/// automatically derived from `RaftNetworkV2` via blanket implementation.
///
/// Direct implementation is an advanced option for fine-grained control.
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
