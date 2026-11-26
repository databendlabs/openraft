use std::time::Duration;

use crate::RaftTypeConfig;
use crate::errors::RPCError;

/// Why a heartbeat got no reply.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HeartbeatFailure<C>
where C: RaftTypeConfig
{
    /// The network returned an error before any response.
    #[error(transparent)]
    Rpc(RPCError<C>),

    /// The response stream ended without a response.
    #[error("the response stream ended without a response")]
    NoResponse,

    /// No response arrived within this heartbeat timeout.
    #[error("no response within {0:?}")]
    Timeout(Duration),
}
