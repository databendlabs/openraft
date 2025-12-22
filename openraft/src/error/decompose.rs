//! Utilities for decomposing error types.

use std::error::Error;

use crate::RaftTypeConfig;
use crate::error::Infallible;
use crate::error::RPCError;
use crate::error::RaftError;
use crate::error::Unreachable;
use crate::error::into_ok::into_ok;

/// Simplifies error handling by extracting the inner error from a composite error.
///
/// This trait helps handle nested error types common in Raft operations where
/// errors can be either recoverable API errors or fatal system errors.
///
/// It converts `Result<R, CompositeError>` to `Result<Result<R, Self::InnerError>, OuterError>`,
/// where `CompositeError` is a composite of `Self::InnerError` and `OuterError`.
///
/// # Implementations
///
/// - `Result<R, RaftError<C, E>>` decomposes into `Result<Result<R, E>, RaftError<C>>`, separating
///   API errors (`E`) from fatal errors.
///
/// - `Result<R, RPCError<C, RaftError<C, E>>>` decomposes into `Result<Result<R, E>, RPCError<C>>`,
///   separating API errors from transport and fatal errors. Note: `RaftError::Fatal` is converted
///   to `RPCError::Unreachable`.
///
/// # Example: Handling RaftError
///
/// ```ignore
/// use openraft::error::{DecomposeResult, RaftError, ClientWriteError};
///
/// async fn handle_write(raft: &Raft<Config>, request: Request) -> Result<(), AppError> {
///     let result = raft.client_write(request).await;
///
///     // Decompose separates Fatal errors from API errors
///     match result.decompose() {
///         Ok(Ok(response)) => {
///             // Success - process the response
///             Ok(())
///         }
///         Ok(Err(ClientWriteError::ForwardToLeader(fwd))) => {
///             // Recoverable: forward request to the current leader
///             Err(AppError::NotLeader(fwd.leader_id))
///         }
///         Ok(Err(ClientWriteError::ChangeMembershipError(_))) => {
///             // Recoverable: membership change in progress, retry later
///             Err(AppError::RetryLater)
///         }
///         Err(RaftError::Fatal(fatal)) => {
///             // Fatal: Raft node is shutting down or storage failed
///             Err(AppError::RaftStopped(fatal))
///         }
///         Err(RaftError::APIError(_)) => {
///             // This branch is unreachable after decompose()
///             unreachable!()
///         }
///     }
/// }
/// ```
///
/// # Example: Handling RPCError
///
/// ```ignore
/// use openraft::error::{DecomposeResult, RPCError};
///
/// async fn forward_to_leader<C: RaftTypeConfig>(
///     network: &mut Network,
///     request: Request
/// ) -> Result<Response, AppError> {
///     let result: Result<Response, RPCError<C, RaftError<C, ClientWriteError<C>>>>
///         = network.send_request(request).await;
///
///     match result.decompose() {
///         Ok(Ok(response)) => Ok(response),
///         Ok(Err(api_error)) => {
///             // Remote node returned an API error (e.g., ForwardToLeader)
///             Err(AppError::from(api_error))
///         }
///         Err(RPCError::Timeout(_)) => Err(AppError::Timeout),
///         Err(RPCError::Unreachable(_)) => Err(AppError::NodeDown),
///         Err(RPCError::Network(_)) => Err(AppError::NetworkError),
///         Err(RPCError::RemoteError(_)) => {
///             // This branch is unreachable after decompose()
///             unreachable!()
///         }
///     }
/// }
/// ```
pub trait DecomposeResult<C, R, OuterError>
where C: RaftTypeConfig
{
    /// The inner error type extracted from the composite error.
    type InnerError;

    /// Decompose a composite error into its inner and outer components.
    ///
    /// Returns:
    /// - `Ok(Ok(r))` if the original result was successful
    /// - `Ok(Err(inner))` if the error was an inner/API error
    /// - `Err(outer)` if the error was an outer/fatal error
    fn decompose(self) -> Result<Result<R, Self::InnerError>, OuterError>;

    /// Convert `Result<R, CompositeErr>` to `Result<R, OuterError>`,
    /// when `Self::InnerError` is an infallible type.
    ///
    /// This is useful when the API error type is `Infallible`, meaning only
    /// outer errors can occur.
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
