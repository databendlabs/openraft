use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::errors::ClientWriteError;
use crate::raft::ClientWriteResponse;
use crate::raft::ClientWriteResult;
use crate::type_config::alias::LogIdOf;

/// The result of a write operation, returned by [`Raft::client_write_many()`].
///
/// This uses [`ClientWriteError`] so callers can distinguish a request rejected before append
/// from one whose outcome became unknown after append. Batch writes only produce
/// [`ClientWriteError::ForwardToLeader`] and [`ClientWriteError::LogEntryDiscarded`].
///
/// [`Raft::client_write_many()`]: crate::Raft::client_write_many
#[since(version = "0.10.0", change = "changed the error type to `ClientWriteError<C>`")]
pub type WriteResult<C> = Result<WriteResponse<C>, ClientWriteError<C>>;

/// Response from a successful write operation.
///
/// This is a simplified version of [`ClientWriteResponse`] used by
/// [`Raft::client_write_many()`]. It contains the log ID where the entry
/// was applied and the application-defined response.
///
/// [`Raft::client_write_many()`]: crate::Raft::client_write_many
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound = "C::R: crate::AppDataResponse")
)]
pub struct WriteResponse<C: RaftTypeConfig> {
    /// The log ID of the applied entry.
    pub log_id: LogIdOf<C>,

    /// Application-defined response data.
    pub response: C::R,
}

impl<C: RaftTypeConfig> From<ClientWriteResponse<C>> for WriteResponse<C> {
    fn from(resp: ClientWriteResponse<C>) -> Self {
        WriteResponse {
            log_id: resp.log_id,
            response: resp.data,
        }
    }
}

/// Convert `ClientWriteResult` to `WriteResult`.
pub(crate) fn into_write_result<C: RaftTypeConfig>(result: ClientWriteResult<C>) -> WriteResult<C> {
    match result {
        Ok(resp) => Ok(resp.into()),
        Err(e @ ClientWriteError::ForwardToLeader(_)) => Err(e),
        Err(e @ ClientWriteError::LogEntryDiscarded(_)) => Err(e),
        Err(ClientWriteError::ChangeMembershipError(_)) => {
            unreachable!("ChangeMembershipError should not occur for normal writes")
        }
        Err(ClientWriteError::PreconditionFailed(_)) => {
            unreachable!("PreconditionFailed should not occur for normal writes")
        }
    }
}
