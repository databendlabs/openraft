use std::fmt;

use crate::display_ext::DisplayOptionExt;
use crate::LogId;
use crate::RaftTypeConfig;
use crate::Vote;

#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct TransferLeaderRequest<C>
where C: RaftTypeConfig
{
    /// The vote of the Leader that is transferring the leadership.
    pub(crate) from: Vote<C::NodeId>,

    /// The assigned node to be the next Leader.
    pub(crate) to: C::NodeId,

    /// The last log id the `to` node should at least have to become Leader.
    pub(crate) last_log_id: Option<LogId<C::NodeId>>,
}

impl<C> TransferLeaderRequest<C>
where C: RaftTypeConfig
{
    pub fn new(from: Vote<C::NodeId>, to: C::NodeId, last_log_id: Option<LogId<C::NodeId>>) -> Self {
        Self { from, to, last_log_id }
    }

    pub fn from(&self) -> &Vote<C::NodeId> {
        &self.from
    }

    pub fn to(&self) -> &C::NodeId {
        &self.to
    }

    pub fn last_log_id(&self) -> Option<&LogId<C::NodeId>> {
        self.last_log_id.as_ref()
    }
}

impl<C> fmt::Display for TransferLeaderRequest<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(from={}, to={}, last_log_id={})",
            self.from,
            self.to,
            self.last_log_id.display()
        )
    }
}
