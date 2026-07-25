//! Post-hook types for intercepting RPC responses after they are received.

use openraft::base::BoxFuture;
use openraft::errors::RPCError;
use openraft_memstore::MemNodeId;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemRpcRequest;
use crate::fixtures::MemRpcResponse;
use crate::fixtures::TypedRaftRouter;

/// Hook function called after an RPC response is received from a target node.
///
/// Arguments: `(router, request, response, from_id, to_id)`
pub type PostHook = Box<
    dyn Fn(&TypedRaftRouter, MemRpcRequest, MemRpcResponse, MemNodeId, MemNodeId) -> PostHookResult + Send + 'static,
>;

/// Result type for post-hook functions.
pub type PostHookResult = BoxFuture<'static, Result<(), RPCError<TypeConfig>>>;
