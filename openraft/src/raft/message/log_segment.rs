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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::testing::blank_ent;

    type Seg = LogSegment<UTConfig>;

    #[test]
    fn test_last_empty_no_prev() {
        let seg = Seg::new(None, vec![]);
        assert_eq!(None, seg.last());
    }

    #[test]
    fn test_last_empty_with_prev() {
        let seg = Seg::new(Some(log_id(1, 1, 5)), vec![]);
        assert_eq!(Some(log_id(1, 1, 5)), seg.last());
    }

    #[test]
    fn test_last_with_entries() {
        let seg = Seg::new(Some(log_id(1, 1, 5)), vec![
            blank_ent(2, 1, 6),
            blank_ent(2, 1, 7),
        ]);
        assert_eq!(Some(log_id(2, 1, 7)), seg.last());
    }

    #[test]
    fn test_display() {
        let seg = Seg::new(Some(log_id(1, 1, 5)), vec![blank_ent(2, 1, 6)]);
        assert_eq!("prev:T1-N1.5, entries:[T2-N1.6:blank]", seg.to_string());
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
