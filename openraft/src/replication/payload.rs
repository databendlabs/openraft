use std::fmt;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::log_id_range::LogIdRange;
use crate::type_config::alias::LogIdOf;

/// The payload specifying which logs to replicate.
#[derive(PartialEq, Eq, Clone, Debug)]
pub(crate) enum Payload<C>
where C: RaftTypeConfig
{
    /// Probe logs from a fixed candidate range `(prev, last]`.
    ///
    /// The replication stream sends one non-empty AppendEntries request. Count and storage byte
    /// limits may reduce it to a prefix of this range; the engine then recomputes the next probe
    /// from the acknowledged prefix.
    LogIdRange { log_id_range: LogIdRange<C> },

    /// Replicate logs after `prev` with no upper bound.
    ///
    /// Used for streaming replication where the leader continuously sends new logs.
    /// The `prev` is updated as logs are acknowledged.
    LogsSince { prev: Option<LogIdOf<C>> },
}

impl<C> fmt::Display for Payload<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self {
            Payload::LogIdRange { log_id_range } => {
                write!(f, "LogIdRange{{{}}}", log_id_range)
            }
            Payload::LogsSince { prev } => {
                write!(f, "LogsSince{{{}}}", prev.display(),)
            }
        }
    }
}

impl<C> Payload<C>
where C: RaftTypeConfig
{
    pub(crate) fn update_matching(&mut self, matching: Option<LogIdOf<C>>) {
        match self {
            Payload::LogIdRange { log_id_range } => log_id_range.prev = matching,
            Payload::LogsSince { prev } => *prev = matching,
        }
    }

    pub(crate) fn len(&self) -> Option<u64> {
        match self {
            Payload::LogIdRange { log_id_range } => Some(log_id_range.len()),
            Payload::LogsSince { .. } => None,
        }
    }
}
