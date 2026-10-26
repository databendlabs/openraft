//! RPC error type definitions for test fixtures.

use openraft::RaftTypeConfig;
use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use openraft::errors::Unreachable;

use crate::fixtures::Direction;

/// Type of RPC error to inject during testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcErrorType {
    /// Returns [`Unreachable`](`openraft::error::Unreachable`).
    Unreachable,
    /// Returns [`NetworkError`](`openraft::error::NetworkError`).
    NetworkError,
}

impl RpcErrorType {
    pub fn make_error<C, E>(&self, id: C::NodeId, dir: Direction) -> RPCError<C, E>
    where
        C: RaftTypeConfig,
        E: std::error::Error,
    {
        let msg = format!("error {} id={}", dir, id);

        match self {
            RpcErrorType::Unreachable => Unreachable::<C>::from_string(msg).into(),
            RpcErrorType::NetworkError => NetworkError::<C>::from_string(msg).into(),
        }
    }
}
