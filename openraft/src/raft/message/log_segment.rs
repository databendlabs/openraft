use std::fmt;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::DisplaySliceExt;
use crate::entry::RaftEntry;
use crate::type_config::alias::LogIdOf;

/// A contiguous segment of log entries, anchored by the entry immediately preceding it.
///
/// - `prev_log_id`: the log id of the entry just before this segment; `None` means the segment
///   starts at the very beginning of the log.
/// - `entries`: the entries in this segment, each consecutive to the previous.
pub struct LogSegment<C: RaftTypeConfig> {
    pub prev_log_id: Option<LogIdOf<C>>,
    pub entries: Vec<C::Entry>,
}

impl<C: RaftTypeConfig> LogSegment<C> {
    pub fn new(prev_log_id: Option<LogIdOf<C>>, entries: Vec<C::Entry>) -> Self {
        Self { prev_log_id, entries }
    }

    /// Returns the log id of the last entry in this segment, or `prev_log_id` if empty.
    pub fn last(&self) -> Option<LogIdOf<C>> {
        self.entries.last().map(|e| e.log_id()).or_else(|| self.prev_log_id.clone())
    }
}

impl<C: RaftTypeConfig> fmt::Display for LogSegment<C>
where C::Entry: fmt::Display
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "prev:{}, entries:{}",
            self.prev_log_id.display(),
            self.entries.display()
        )
    }
}
