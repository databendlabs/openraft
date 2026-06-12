//! Defines the [`NetVote`] trait for Vote RPC.

use openraft_macros::add_async_trait;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::errors::RPCError;
use crate::network::RPCOption;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;

/// Sends Vote RPCs to a target node.
///
/// **For most applications, implement [`RaftNetworkV2`] instead.** This trait is
/// automatically derived from `RaftNetworkV2` via blanket implementation.
///
/// Direct implementation is an advanced option for fine-grained control.
///
/// [`RaftNetworkV2`]: crate::network::RaftNetworkV2
#[add_async_trait]
pub trait NetVote<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send a RequestVote RPC to the target.
    async fn vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>>;

    /// Send a Pre-Vote RPC to the target.
    ///
    /// The default implementation synthesizes a **granting** response, so a network that has not
    /// implemented `pre_vote` makes Pre-Vote a no-op and elections proceed as before. A transport
    /// failure must instead be returned as `Err` (it is not counted as a grant). See
    /// [`RaftNetworkV2::pre_vote`](crate::network::v2::RaftNetworkV2::pre_vote).
    async fn pre_vote(&mut self, rpc: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        Ok(VoteResponse::new(rpc.vote, None, true))
    }
}
