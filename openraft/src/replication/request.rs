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
    Data {
        inflight_id: Option<InflightId>,
        data: Data<C>,
    },
}

impl<C> Replicate<C>
where C: RaftTypeConfig
{
    pub(crate) fn logs(log_id_range: LogIdRange<C>, inflight_id: Option<InflightId>) -> Self {
        Self::Data {
            inflight_id,
            data: Data::new_logs(log_id_range),
        }
    }
}

impl<C: RaftTypeConfig> fmt::Display for Replicate<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Committed { committed } => write!(f, "Committed({})", committed.display(),),
            Self::Data { inflight_id, data } => {
                write!(f, "Data({}), inflight_id:{}", data, inflight_id.display())
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
/// It defines what data to send to a follower/learner and an id to identify who is sending this
/// data.
/// The data is either a series of logs or a snapshot.
#[derive(PartialEq, Eq)]
pub(crate) enum Data<C>
where C: RaftTypeConfig
{
    Committed,
    Logs(LogIdRange<C>),
}

impl<C> fmt::Debug for Data<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Data::Committed => {
                write!(f, "Data::Committed")
            }
            Self::Logs(l) => f.debug_struct("Data::Logs").field("log_id_range", l).finish(),
        }
    }
}

impl<C: RaftTypeConfig> fmt::Display for Data<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Data::Committed => {
                write!(f, "Committed")
            }
            Self::Logs(l) => {
                write!(f, "Logs{{log_id_range: {}}}", l)
            }
        }
    }
}

impl<C> Data<C>
where C: RaftTypeConfig
{
    pub(crate) fn new_committed() -> Self {
        Self::Committed
    }

    pub(crate) fn new_logs(log_id_range: LogIdRange<C>) -> Self {
        Self::Logs(log_id_range)
    }

    /// Return true if the data includes any payload, i.e., not a heartbeat.
    pub(crate) fn has_payload(&self) -> bool {
        match self {
            Self::Committed => false,
            Self::Logs(_) => true,
        }
    }
}
