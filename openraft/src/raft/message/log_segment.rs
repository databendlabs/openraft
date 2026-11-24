use std::fmt;

use display_more::DisplayOptionExt;
use display_more::DisplaySliceExt;

use crate::RaftTypeConfig;
use crate::entry::RaftEntry;
use crate::type_config::alias::LogIdOf;

/// A contiguous segment of log entries, anchored by the entry immediately preceding it.
pub struct LogSegment<C>
where C: RaftTypeConfig
{
    /// The log id of the entry just before this segment; `None` means the segment starts at the
    /// very beginning of the log.
    pub prev_log_id: Option<LogIdOf<C>>,
    /// The entries in this segment, each consecutive to the previous.
    pub entries: Vec<C::Entry>,
}

impl<C> LogSegment<C>
where C: RaftTypeConfig
{
    /// Creates a segment of `entries` that follows the entry at `prev_log_id`.
    pub fn new(prev_log_id: Option<LogIdOf<C>>, entries: Vec<C::Entry>) -> Self {
        Self { prev_log_id, entries }
    }

    /// Returns the log id of the last entry in this segment, or `prev_log_id` if empty.
    pub fn last(&self) -> Option<LogIdOf<C>> {
        let Some(last_entry) = self.entries.last() else {
            return self.prev_log_id.clone();
        };
        let log_id = last_entry.log_id();
        Some(log_id)
    }
}

impl<C> fmt::Display for LogSegment<C>
where C: RaftTypeConfig
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
        let got = seg.last();
        assert_eq!(None, got);
    }

    #[test]
    fn test_last_empty_with_prev() {
        let seg = Seg::new(Some(log_id(1, 1, 5)), vec![]);
        let want = Some(log_id(1, 1, 5));
        let got = seg.last();
        assert_eq!(want, got);
    }

    #[test]
    fn test_last_with_entries() {
        let seg = Seg::new(Some(log_id(1, 1, 5)), vec![
            blank_ent::<UTConfig>(2, 1, 6),
            blank_ent::<UTConfig>(2, 1, 7),
        ]);
        let want = Some(log_id(2, 1, 7));
        let got = seg.last();
        assert_eq!(want, got);
    }

    #[test]
    fn test_display() {
        let seg = Seg::new(Some(log_id(1, 1, 5)), vec![blank_ent::<UTConfig>(2, 1, 6)]);
        let got = seg.to_string();
        assert_eq!("prev:T1-N1.5, entries:[T2-N1.6:blank]", got);
    }
}
