use crate::RaftTypeConfig;
use crate::error::ClientWriteError;
use crate::error::ForwardToLeader;
use crate::raft::ClientWriteResponse;
use crate::raft::ClientWriteResult;
use crate::type_config::alias::LogIdOf;

pub type WriteResult<C> = Result<WriteResponse<C>, ForwardToLeader<C>>;

#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound = "C::R: crate::AppDataResponse")
)]
pub struct WriteResponse<C: RaftTypeConfig> {
    /// The id of the log that is applied.
    pub log_id: LogIdOf<C>,

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
pub fn into_write_result<C: RaftTypeConfig>(result: ClientWriteResult<C>) -> WriteResult<C> {
    match result {
        Ok(resp) => Ok(resp.into()),
        Err(ClientWriteError::ForwardToLeader(e)) => Err(e),
        Err(ClientWriteError::ChangeMembershipError(_)) => {
            unreachable!("ChangeMembershipError should not occur for normal writes")
        }
    }
}
