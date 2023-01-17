use std::fmt::Display;
use std::fmt::Formatter;

use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;

/// A log id range of continuous series of log entries.
///
/// The range of log to send is left open right close: `(prev_log_id, last_log_id]`.
#[derive(Clone, Copy, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct LogIdRange<NID: NodeId> {
    /// The prev log id before the first to send, exclusive.
    pub(crate) prev_log_id: Option<LogId<NID>>,

    /// The last log id to send, inclusive.
    pub(crate) last_log_id: Option<LogId<NID>>,
}

impl<NID: NodeId> Display for LogIdRange<NID> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}]", self.prev_log_id.summary(), self.last_log_id.summary())
    }
}

impl<NID: NodeId> LogIdRange<NID> {
    pub(crate) fn new(prev: Option<LogId<NID>>, last: Option<LogId<NID>>) -> Self {
        Self {
            prev_log_id: prev,
            last_log_id: last,
        }
    }
}
