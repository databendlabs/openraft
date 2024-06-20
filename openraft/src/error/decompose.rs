use std::error::Error;

use crate::error::into_ok::into_ok;
use crate::error::Fatal;
use crate::error::Infallible;
use crate::error::RPCError;
use crate::error::RaftError;
use crate::error::StreamingError;
use crate::error::Unreachable;
use crate::RaftTypeConfig;

/// Simplifies error handling by extracting the inner error from a composite error.
/// For example, converting `Result<R, CompositeError>`
/// to `Result<Result<R, Self::InnerError>, OuterError>`,
/// where `SomeCompositeError` is a composite of `Self::InnerError` and `OuterError`.
pub trait DecomposeResult<C, R, OuterError>
where C: RaftTypeConfig
{
    type InnerError;

    fn decompose(self) -> Result<Result<R, Self::InnerError>, OuterError>;

    /// Convert `Result<R, CompositeErr>`
    /// to `Result<R, E>`,
    /// if `Self::InnerError` is a infallible type.
    fn decompose_infallible(self) -> Result<R, OuterError>
    where
        Self::InnerError: Into<Infallible>,
        Self: Sized,
    {
        self.decompose().map(into_ok)
    }
}

impl<C, R, E> DecomposeResult<C, R, RaftError<C::NodeId>> for Result<R, RaftError<C::NodeId, E>>
where C: RaftTypeConfig
{
    type InnerError = E;

    fn decompose(self) -> Result<Result<R, E>, RaftError<C::NodeId>> {
        match self {
            Ok(r) => Ok(Ok(r)),
            Err(e) => match e {
                RaftError::APIError(e) => Ok(Err(e)),
                RaftError::Fatal(e) => Err(RaftError::Fatal(e)),
            },
        }
    }
}

impl<C, R, E> DecomposeResult<C, R, RPCError<C::NodeId, C::Node>>
    for Result<R, RPCError<C::NodeId, C::Node, RaftError<C::NodeId, E>>>
where
    C: RaftTypeConfig,
    E: Error,
{
    type InnerError = E;

    /// `RaftError::Fatal` is considered as `RPCError::Unreachable`.
    fn decompose(self) -> Result<Result<R, E>, RPCError<C::NodeId, C::Node>> {
        match self {
            Ok(r) => Ok(Ok(r)),
            Err(e) => match e {
                RPCError::Timeout(e) => Err(RPCError::Timeout(e)),
                RPCError::Unreachable(e) => Err(RPCError::Unreachable(e)),
                RPCError::PayloadTooLarge(e) => Err(RPCError::PayloadTooLarge(e)),
                RPCError::Network(e) => Err(RPCError::Network(e)),
                RPCError::RemoteError(e) => match e.source {
                    RaftError::APIError(e) => Ok(Err(e)),
                    RaftError::Fatal(e) => Err(RPCError::Unreachable(Unreachable::new(&e))),
                },
            },
        }
    }
}

impl<C, R> DecomposeResult<C, R, StreamingError<C>> for Result<R, StreamingError<C, Fatal<C::NodeId>>>
where C: RaftTypeConfig
{
    type InnerError = Infallible;

    /// `Fatal` is considered as `RPCError::Unreachable`.
    fn decompose(self) -> Result<Result<R, Self::InnerError>, StreamingError<C>> {
        match self {
            Ok(r) => Ok(Ok(r)),
            Err(e) => match e {
                StreamingError::Closed(e) => Err(StreamingError::Closed(e)),
                StreamingError::StorageError(e) => Err(StreamingError::StorageError(e)),
                StreamingError::Timeout(e) => Err(StreamingError::Timeout(e)),
                StreamingError::Unreachable(e) => Err(StreamingError::Unreachable(e)),
                StreamingError::Network(e) => Err(StreamingError::Network(e)),
                StreamingError::RemoteError(e) => Err(StreamingError::Unreachable(Unreachable::new(&e.source))),
            },
        }
    }
}
