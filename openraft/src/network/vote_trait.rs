//! Defines the [`RaftNetworkVote`] trait for Vote RPC.

use openraft_macros::add_async_trait;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::error::RPCError;
use crate::network::RPCOption;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;

/// Sends Vote RPCs to a target node.
///
/// This trait is part of the granular network API. It can be implemented directly
/// or automatically via blanket impl when implementing [`RaftNetworkV2`].
///
/// [`RaftNetworkV2`]: crate::network::RaftNetworkV2
#[add_async_trait]
pub trait RaftNetworkVote<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send a RequestVote RPC to the target.
    async fn vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>>;
}
