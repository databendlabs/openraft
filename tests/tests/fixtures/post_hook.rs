//! Post-hook types for intercepting RPC responses after they are received.

use openraft::base::BoxFuture;
use openraft::error::RPCError;
use openraft_memstore::MemNodeId;
use openraft_memstore::TypeConfig;

use crate::fixtures::TypedRaftRouter;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::rpc_response::RpcResponse;

/// Hook function called after an RPC response is received from a target node.
///
/// Arguments: `(router, request, response, from_id, to_id)`
pub type PostHook = Box<
    dyn Fn(&TypedRaftRouter, RpcRequest<TypeConfig>, RpcResponse<TypeConfig>, MemNodeId, MemNodeId) -> PostHookResult
        + Send
        + 'static,
>;

/// Result type for post-hook functions.
pub type PostHookResult = BoxFuture<'static, Result<(), RPCError<TypeConfig>>>;
