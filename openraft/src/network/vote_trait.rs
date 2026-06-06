//! Defines the [`NetVote`] trait for Vote RPC.

use anyerror::AnyError;
use openraft_macros::add_async_trait;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::errors::RPCError;
use crate::errors::Unreachable;
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

    /// Send a pre-vote probe RPC to the target (Raft §9.6).
    ///
    /// Defaults to [`Unreachable`], which the caller counts as a grant — so a
    /// network that has not implemented pre-vote does not block elections
    /// (rolling-upgrade safe). Most applications implement [`RaftNetworkV2`],
    /// which carries the real `pre_vote` and forwards to it via blanket impl.
    ///
    /// [`RaftNetworkV2`]: crate::network::RaftNetworkV2
    async fn pre_vote(&mut self, _rpc: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        Err(RPCError::Unreachable(Unreachable::new(&AnyError::error(
            "pre_vote not implemented",
        ))))
    }
}
