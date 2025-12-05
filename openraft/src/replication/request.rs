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

/// The payload specifying which logs to replicate.
#[derive(PartialEq, Eq, Clone, Debug)]
pub(crate) enum DataPayload<C>
where C: RaftTypeConfig
{
    /// Replicate logs in a fixed range `(prev, last]`.
    ///
    /// Used for batch replication where the leader sends a known set of log entries.
    LogIdRange { log_id_range: LogIdRange<C> },

    /// Replicate logs after `prev` with no upper bound.
    ///
    /// Used for streaming replication where the leader continuously sends new logs.
    /// The `prev` is updated as logs are acknowledged.
    LogsSince { prev: Option<LogIdOf<C>> },
}

/// A replication data request containing log entries to send.
///
/// Each request has a corresponding `Inflight` record on the leader, identified by an
/// `InflightId`. The follower's response carries the same `InflightId` so the leader can
/// match the response to the correct inflight state.
#[derive(PartialEq, Eq, Clone, Debug)]
pub(crate) struct Data<C>
where C: RaftTypeConfig
{
    /// Identifies this inflight request for matching with responses.
    pub(crate) inflight_id: InflightId,

    /// Specifies which logs to replicate.
    pub(crate) payload: DataPayload<C>,
}

impl<C> Default for Data<C>
where C: RaftTypeConfig
{
    fn default() -> Self {
        Data {
            inflight_id: InflightId::new(0),
            payload: DataPayload::LogIdRange {
                log_id_range: LogIdRange::new(None, None),
            },
        }
    }
}

impl<C: RaftTypeConfig> fmt::Display for Data<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.payload {
            DataPayload::LogIdRange { log_id_range } => {
                write!(
                    f,
                    "Data{{log_id_range: {}, inflight_id: {}}}",
                    log_id_range, self.inflight_id
                )
            }
            DataPayload::LogsSince { prev } => {
                write!(
                    f,
                    "Data{{logs_since: {}, inflight_id: {}}}",
                    prev.display(),
                    self.inflight_id
                )
            }
        }
    }
}

impl<C> Data<C>
where C: RaftTypeConfig
{
    /// Creates a request to replicate logs in a fixed range.
    pub(crate) fn new_logs(log_id_range: LogIdRange<C>, inflight_id: InflightId) -> Self {
        Self {
            inflight_id,
            payload: DataPayload::LogIdRange { log_id_range },
        }
    }

    /// Creates a request to replicate logs after `prev` with no upper bound.
    pub(crate) fn new_logs_since(prev: Option<LogIdOf<C>>, inflight_id: InflightId) -> Self {
        Self {
            inflight_id,
            payload: DataPayload::LogsSince { prev },
        }
    }
}
