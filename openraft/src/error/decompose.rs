//! Utilities for decomposing error types.

use std::error::Error;

use crate::RaftTypeConfig;
use crate::error::Infallible;
use crate::error::RPCError;
use crate::error::RaftError;
use crate::error::Unreachable;
use crate::error::into_ok::into_ok;

/// Simplifies error handling by extracting the inner error from a composite error.
/// For example, converting `Result<R, CompositeError>`
/// to `Result<Result<R, Self::InnerError>, OuterError>`,
/// where `SomeCompositeError` is a composite of `Self::InnerError` and `OuterError`.
pub trait DecomposeResult<C, R, OuterError>
where C: RaftTypeConfig
{
    /// The inner error type extracted from the composite error.
    type InnerError;

    /// Decompose a composite error into its inner and outer components.
    fn decompose(self) -> Result<Result<R, Self::InnerError>, OuterError>;

    /// Convert `Result<R, CompositeErr>`
    /// to `Result<R, E>`,
    /// if `Self::InnerError` is an infallible type.
    fn decompose_infallible(self) -> Result<R, OuterError>
    where
        Self::InnerError: Into<Infallible>,
        Self: Sized,
    {
        self.decompose().map(into_ok)
    }
}

impl<C, R, E> DecomposeResult<C, R, RaftError<C>> for Result<R, RaftError<C, E>>
where C: RaftTypeConfig
{
    type InnerError = E;

    fn decompose(self) -> Result<Result<R, E>, RaftError<C>> {
        match self {
            Ok(r) => Ok(Ok(r)),
            Err(e) => match e {
                RaftError::APIError(e) => Ok(Err(e)),
                RaftError::Fatal(e) => Err(RaftError::Fatal(e)),
            },
        }
    }
}

impl<C, R, E> DecomposeResult<C, R, RPCError<C>> for Result<R, RPCError<C, RaftError<C, E>>>
where
    C: RaftTypeConfig,
    E: Error,
{
    type InnerError = E;

    /// `RaftError::Fatal` is considered as `RPCError::Unreachable`.
    fn decompose(self) -> Result<Result<R, E>, RPCError<C>> {
        match self {
            Ok(r) => Ok(Ok(r)),
            Err(e) => match e {
                RPCError::Timeout(e) => Err(RPCError::Timeout(e)),
                RPCError::Unreachable(e) => Err(RPCError::Unreachable(e)),
                RPCError::Network(e) => Err(RPCError::Network(e)),
                RPCError::RemoteError(e) => match e.source {
                    RaftError::APIError(e) => Ok(Err(e)),
                    RaftError::Fatal(e) => Err(RPCError::Unreachable(Unreachable::new(&e))),
                },
            },
        }
    }
}
