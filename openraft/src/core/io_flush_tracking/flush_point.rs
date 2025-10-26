use std::fmt;

use crate::LogId;
use crate::RaftTypeConfig;
use crate::Vote;
use crate::display_ext::display_option::DisplayOptionExt;

/// State of the most recently flushed log I/O operation.
///
/// This represents the durable state of the log storage after a flush completes. It includes:
/// - The vote(leader) under which the I/O was submitted
/// - The last log entry that was written (if any logs were appended)
///
/// # Ordering
///
/// Ordered lexicographically as `(vote, last_log_id)`:
/// - Higher term always > lower term
/// - Same term: higher node_id > lower node_id (for non-committed votes)
/// - Same vote: longer log (higher index) > shorter log
///
/// This enables `wait_until_ge()` to wait for specific progress milestones.
///
/// # Special Cases
///
/// - `!vote.is_committed() && last_log_id.is_none()`: A candidate's vote is accepted but it has not
///   yet become leader (no AppendEntries received).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd)]
pub struct FlushPoint<C>
where C: RaftTypeConfig
{
    /// The vote(leader) under which this I/O operation was submitted.
    pub vote: Vote<C>,

    /// The last log entry that was flushed, or `None` if only a vote was saved without appending
    /// logs.
    pub last_log_id: Option<LogId<C>>,
}

impl<C> fmt::Display for FlushPoint<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FlushPoint({}, {})", self.vote, self.last_log_id.display(),)
    }
}

impl<C> FlushPoint<C>
where C: RaftTypeConfig
{
    pub fn new(vote: Vote<C>, last_log_id: Option<LogId<C>>) -> Self {
        Self { vote, last_log_id }
    }
}
