use std::fmt;

use crate::type_config::alias::LogIdOf;

/// A replication request sent by RaftCore leader state to replication stream.
#[derive(Debug)]
#[derive(PartialEq, Eq)]
pub(crate) enum Replicate<C>
where C: RaftTypeConfig
{
    /// Inform the replication stream to forward the committed log id to followers/learners.
    Committed { committed: Option<LogIdOf<C>> },

    /// Send a chunk of data, e.g., logs or snapshot.
    Data { data: Data<C> },
}

impl<C> Replicate<C>
where C: RaftTypeConfig
{
    pub(crate) fn logs(log_id_range: LogIdRange<C>, inflight_id: InflightId) -> Self {
        Self::Data {
            data: Data::new_logs(log_id_range, inflight_id),
        }
    }

    pub(crate) fn inflight_id(&self) -> Option<InflightId> {
        match self {
            Replicate::Committed { .. } => None,
            Replicate::Data { data } => Some(data.inflight_id),
        }
    }
}

impl<C: RaftTypeConfig> fmt::Display for Replicate<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Committed { committed } => write!(f, "Committed({})", committed.display(),),
            Self::Data { data } => {
                write!(f, "Data({})", data)
            }
        }
    }
}

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::log_id_range::LogIdRange;
use crate::progress::inflight_id::InflightId;

/// Request to replicate a chunk of data, logs or snapshot.
///
/// The data is either a series of logs or a notification of the committed index.
///
/// - `Committed`: An RPC to synchronize the committed index without log payload. This type of
///   request has no corresponding `Inflight` record on the leader because there's nothing to
///   acknowledge.
///
/// - `Logs`: An RPC that carries actual log entries. Each such request has a corresponding
///   `Inflight` record on the leader, identified by an `InflightId`. The follower's response
///   carries the same `InflightId` so the leader can match the response to the correct inflight
///   state.
#[derive(PartialEq, Eq, Clone, Debug)]
pub(crate) struct Data<C>
where C: RaftTypeConfig
{
    pub(crate) inflight_id: InflightId,
    pub(crate) log_id_range: LogIdRange<C>,
}

impl<C> Default for Data<C>
where C: RaftTypeConfig
{
    fn default() -> Self {
        Data {
            inflight_id: InflightId::new(0),
            log_id_range: LogIdRange::new(None, None),
        }
    }
}

impl<C: RaftTypeConfig> fmt::Display for Data<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Data{{log_id_range: {}, inflight_id: {}}}",
            self.log_id_range, self.inflight_id
        )
    }
}

impl<C> Data<C>
where C: RaftTypeConfig
{
    pub(crate) fn new_logs(log_id_range: LogIdRange<C>, inflight_id: InflightId) -> Self {
        Self {
            log_id_range,
            inflight_id,
        }
    }
}
