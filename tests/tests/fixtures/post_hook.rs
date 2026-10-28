//! Post-hook types for intercepting RPC responses after they are received.

use openraft::base::BoxFuture;
use openraft::errors::RPCError;
use openraft_memstore::MemNodeId;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemRpcRequest;
use crate::fixtures::MemRpcResponse;
use crate::fixtures::RaftRouter;

/// Owned context for one RPC response the transport is about to return.
///
/// A post-hook future is `'static`, so this context owns the router instead of borrowing it.
pub struct RpcResponseCall {
    /// Router that carries this call.
    pub router: RaftRouter,
    /// Request that produced `response`.
    pub request: MemRpcRequest,
    /// Final transport response.
    pub response: MemRpcResponse,
    /// Node that sent the request.
    pub from: MemNodeId,
    /// Node that produced the response.
    pub to: MemNodeId,
}

/// Stored post-hook. The setter boxes the caller's future into this type.
pub type PostHook = Box<dyn Fn(RpcResponseCall) -> PostHookResult + Send + Sync + 'static>;

/// Result type for post-hook functions.
pub type PostHookResult = BoxFuture<'static, Result<(), RPCError<TypeConfig>>>;
