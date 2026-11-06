//! Defines the [`NetAppend`] trait for AppendEntries RPC.

use openraft_macros::add_async_trait;
use openraft_macros::since;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::errors::RPCError;
use crate::network::RPCOption;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendResult;

/// Sends AppendEntries RPCs to a target node.
///
/// **For most applications, implement [`RaftNetworkV2`] instead.** This trait is
/// automatically derived from `RaftNetworkV2` via blanket implementation.
///
/// Direct implementation is an advanced option for fine-grained control.
///
/// [`RaftNetworkV2`]: crate::network::RaftNetworkV2
#[add_async_trait]
pub trait NetAppend<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send an AppendEntries RPC to the target.
    ///
    /// The network implementation is responsible for enforcing `option.soft_ttl()`.
    ///
    /// See [`RaftNetworkV2::append_entries()`] for the meaning of the result.
    ///
    /// [`RaftNetworkV2::append_entries()`]: crate::network::RaftNetworkV2::append_entries
    #[since(version = "0.10.0", change = "returns AppendResult with an optional conflict hint")]
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>>;
}
