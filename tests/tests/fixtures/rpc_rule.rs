use std::fmt;

use openraft_memstore::MemNodeId;

use crate::fixtures::MemRpcRequest;
use crate::fixtures::rpc_error_type::RpcErrorType;
use crate::fixtures::rpc_kind::RpcKind;

type RequestFilter = Box<dyn Fn(&MemRpcRequest) -> bool + Send>;

/// A directed RPC fault, evaluated in rule insertion order before custom hooks.
pub struct RpcRule {
    from: MemNodeId,
    to: MemNodeId,
    kind: RpcKind,
    /// The local transport error returned when this rule matches.
    pub error: RpcErrorType,
    predicate: Option<RequestFilter>,
}

impl RpcRule {
    /// Fail one RPC kind on one directed link with a network error.
    pub fn new(from: MemNodeId, to: MemNodeId, kind: RpcKind) -> Self {
        Self {
            from,
            to,
            kind,
            error: RpcErrorType::NetworkError,
            predicate: None,
        }
    }

    /// Further restrict the rule using the request's contents.
    pub fn matching<F>(mut self, predicate: F) -> Self
    where F: Fn(&MemRpcRequest) -> bool + Send + 'static {
        self.predicate = Some(Box::new(predicate));
        self
    }

    pub(super) fn matches(&self, request: &MemRpcRequest, from: MemNodeId, to: MemNodeId) -> bool {
        if from != self.from || to != self.to {
            return false;
        }
        let kind = request.kind();
        if kind != self.kind {
            return false;
        }
        if let Some(predicate) = &self.predicate {
            return predicate(request);
        }
        true
    }
}

impl fmt::Debug for RpcRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut rule = f.debug_struct("RpcRule");
        rule.field("from", &self.from);
        rule.field("to", &self.to);
        rule.field("kind", &self.kind);
        rule.field("error", &self.error);
        rule.finish_non_exhaustive()
    }
}
