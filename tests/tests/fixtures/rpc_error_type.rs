//! RPC error type definitions for test fixtures.

use openraft::RaftTypeConfig;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::error::Unreachable;

use crate::fixtures::Direction;

/// Type of RPC error to inject during testing.
#[derive(Debug, Clone, Copy)]
pub enum RpcErrorType {
    /// Returns [`Unreachable`](`openraft::error::Unreachable`).
    Unreachable,
    /// Returns [`NetworkError`](`openraft::error::NetworkError`).
    NetworkError,
}

impl RpcErrorType {
    pub fn make_error<C>(&self, id: C::NodeId, dir: Direction) -> RPCError<C>
    where C: RaftTypeConfig {
        let msg = format!("error {} id={}", dir, id);

        match self {
            RpcErrorType::Unreachable => Unreachable::<C>::from_string(msg).into(),
            RpcErrorType::NetworkError => NetworkError::<C>::from_string(msg).into(),
        }
    }
}
