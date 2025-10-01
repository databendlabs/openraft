use crate::RaftTypeConfig;
use crate::error::Fatal;
use crate::error::Infallible;
use crate::error::RaftError;

/// Convert a `Result<_, Fatal<C>>` to a `Result<T, RaftError<C, E>>`
///
/// This trait is used to convert `Results` from the new nested format(`Result<Result<_,E>,Fatal>`)
/// to a format that is compatible with the older API version(`Result<_, RaftError<E>>`).
/// - In the older Result style, both application Error and Fatal Error are wrapped in the one
///   [`RaftError`] type.
/// - In the new Result style, application Error and Fatal Error are wrapped in the two different
///   types(`Result<_,E>` and `Fatal<C>`).
///
/// The primary use case is for protocol methods that return `Result<T, Fatal<C>>` to be converted
/// to the backward compatible `Result<T, RaftError<C, E>>` which can represent both fatal errors
/// and application-specific errors.
pub(crate) trait IntoRaftResult<C, T, E>
where C: RaftTypeConfig
{
    /// Convert a `Result<Result<T, E>, Fatal<C>>` or `Result<T, Fatal<C>>` to a
    /// `Result<T, RaftError<C, E>>`.
    fn into_raft_result(self) -> Result<T, RaftError<C, E>>;
}

impl<C, T, E> IntoRaftResult<C, T, E> for Result<Result<T, E>, Fatal<C>>
where C: RaftTypeConfig
{
    fn into_raft_result(self) -> Result<T, RaftError<C, E>> {
        match self {
            Ok(Ok(t)) => Ok(t),
            Ok(Err(e)) => Err(RaftError::APIError(e)),
            Err(f) => Err(RaftError::Fatal(f)),
        }
    }
}

impl<C, T> IntoRaftResult<C, T, Infallible> for Result<T, Fatal<C>>
where C: RaftTypeConfig
{
    fn into_raft_result(self) -> Result<T, RaftError<C>> {
        self.map_err(RaftError::Fatal)
    }
}
