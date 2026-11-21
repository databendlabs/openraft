use std::fmt;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::log_id_range::LogIdRange;
use crate::progress::inflight_id::InflightId;
use crate::replication::payload::Payload;
use crate::type_config::alias::LogIdOf;

/// A replication data request containing log entries to send.
///
/// Each request has a corresponding `Inflight` record on the leader, identified by an
/// `InflightId`. The follower's response carries the same `InflightId` so the leader can
/// match the response to the correct inflight state.
#[derive(PartialEq, Eq, Clone, Debug)]
pub(crate) struct Replicate<C>
where C: RaftTypeConfig
{
    /// Identifies this inflight request for matching with responses.
    pub(crate) inflight_id: InflightId,

    /// Specifies which logs to replicate.
    ///
    /// `None` clears the plan the replication task ran before: the task stops reading the log for
    /// it and confirms that to RaftCore with `inflight_id`, see
    /// [`Command::ClearReplicate`](crate::engine::Command::ClearReplicate).
    pub(crate) payload: Option<Payload<C>>,
}

impl<C: RaftTypeConfig> fmt::Display for Replicate<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.payload {
            Some(Payload::Probe { log_id_range }) => {
                write!(
                    f,
                    "Replicate{{probe: {}, inflight_id: {}}}",
                    log_id_range, self.inflight_id
                )
            }
            Some(Payload::LogsSince { prev }) => {
                write!(
                    f,
                    "Replicate{{logs_since: {}, inflight_id: {}}}",
                    prev.display(),
                    self.inflight_id
                )
            }
            None => write!(f, "Replicate{{clear, inflight_id: {}}}", self.inflight_id),
        }
    }
}

impl<C> Replicate<C>
where C: RaftTypeConfig
{
    /// Creates a request to probe the matching point with logs from a candidate range.
    pub(crate) fn new_probe(log_id_range: LogIdRange<C>, inflight_id: InflightId) -> Self {
        Self {
            inflight_id,
            payload: Some(Payload::Probe { log_id_range }),
        }
    }

    /// Creates a request to replicate logs after `prev` with no upper bound.
    pub(crate) fn new_logs_since(prev: Option<LogIdOf<C>>, inflight_id: InflightId) -> Self {
        Self {
            inflight_id,
            payload: Some(Payload::LogsSince { prev }),
        }
    }

    /// Creates a request to clear the plan the replication task ran before, which the task answers
    /// with `inflight_id`.
    pub(crate) fn new_clear(inflight_id: InflightId) -> Self {
        Self {
            inflight_id,
            payload: None,
        }
    }
}
