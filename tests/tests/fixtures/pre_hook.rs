//! Pre-hook types for intercepting RPC calls before they are sent.

use openraft::base::BoxFuture;
use openraft::errors::Infallible;
use openraft::errors::RPCError;
use openraft_memstore::MemNodeId;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemRpcRequest;
use crate::fixtures::RaftRouter;

/// Owned context for one RPC that is about to be sent.
///
/// A pre-hook future is `'static`, so this context owns the router instead of borrowing it.
pub struct RpcCall {
    /// Router that carries this call.
    pub router: RaftRouter,
    /// Request about to be sent.
    pub request: MemRpcRequest,
    /// Node sending the request.
    pub from: MemNodeId,
    /// Node receiving the request.
    pub to: MemNodeId,
}

/// Stored pre-hook. The setter boxes the caller's future into this type.
pub type PreHook = Box<dyn Fn(RpcCall) -> PreHookResult + Send + Sync + 'static>;

/// Result type for pre-hook functions.
///
/// Pre-hooks cannot return remote errors, only local errors.
pub type PreHookResult = BoxFuture<'static, Result<(), RPCError<TypeConfig, Infallible>>>;
