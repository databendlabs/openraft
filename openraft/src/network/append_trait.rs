//! Defines the [`NetAppend`] trait for AppendEntries RPC.

use openraft_macros::add_async_trait;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::errors::RPCError;
use crate::network::RPCOption;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;

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
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>>;
}
