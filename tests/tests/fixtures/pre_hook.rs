//! Pre-hook types for intercepting RPC calls before they are sent.

use openraft::base::BoxFuture;
use openraft::error::Infallible;
use openraft::error::RPCError;
use openraft_memstore::MemNodeId;
use openraft_memstore::TypeConfig;

use crate::fixtures::TypedRaftRouter;
use crate::fixtures::rpc_request::RpcRequest;

/// Hook function called before an RPC is sent to a target node.
///
/// Arguments: `(router, rpc, from_id, to_id)`
pub type PreHook =
    Box<dyn Fn(&TypedRaftRouter, RpcRequest<TypeConfig>, MemNodeId, MemNodeId) -> PreHookResult + Send + 'static>;

/// Result type for pre-hook functions.
///
/// Pre-hooks cannot return remote errors, only local errors.
pub type PreHookResult = BoxFuture<'static, Result<(), RPCError<TypeConfig, Infallible>>>;
