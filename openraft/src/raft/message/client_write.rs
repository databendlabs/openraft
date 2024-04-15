use std::fmt::Debug;

use openraft_macros::since;

use crate::error::ClientWriteError;
use crate::type_config::alias::NodeIdOf;
use crate::type_config::alias::NodeOf;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::RaftTypeConfig;

/// The result of a write request to Raft.
pub type ClientWriteResult<C> = Result<ClientWriteResponse<C>, ClientWriteError<NodeIdOf<C>, NodeOf<C>>>;

/// The response to a client-request.
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound = "C::R: crate::AppDataResponse")
)]
pub struct ClientWriteResponse<C: RaftTypeConfig> {
    /// The id of the log that is applied.
    pub log_id: LogId<C::NodeId>,

    /// Application specific response data.
    pub data: C::R,

    /// If the log entry is a change-membership entry.
    pub membership: Option<Membership<C::NodeId, C::Node>>,
}

impl<C> ClientWriteResponse<C>
where C: RaftTypeConfig
{
    /// Create a new instance of `ClientWriteResponse`.
    #[allow(dead_code)]
    #[since(version = "0.9.5")]
    pub(crate) fn new_app_response(log_id: LogId<C::NodeId>, data: C::R) -> Self {
        Self {
            log_id,
            data,
            membership: None,
        }
    }

    #[since(version = "0.9.5")]
    pub fn log_id(&self) -> &LogId<C::NodeId> {
        &self.log_id
    }

    #[since(version = "0.9.5")]
    pub fn response(&self) -> &C::R {
        &self.data
    }

    /// Return membership config if the log entry is a change-membership entry.
    #[since(version = "0.9.5")]
    pub fn membership(&self) -> &Option<Membership<C::NodeId, C::Node>> {
        &self.membership
    }
}

impl<C: RaftTypeConfig> Debug for ClientWriteResponse<C>
where C::R: Debug
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientWriteResponse")
            .field("log_id", &self.log_id)
            .field("data", &self.data)
            .field("membership", &self.membership)
            .finish()
    }
}

impl<C: RaftTypeConfig> MessageSummary<ClientWriteResponse<C>> for ClientWriteResponse<C> {
    fn summary(&self) -> String {
        format!("log_id: {}, membership: {:?}", self.log_id, self.membership)
    }
}
