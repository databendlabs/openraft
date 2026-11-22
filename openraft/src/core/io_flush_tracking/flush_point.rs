use std::fmt;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::Vote;
use crate::type_config::alias::LogIdOf;

/// State of the most recently flushed log I/O operation.
///
/// This represents the durable state of the log storage after a flush completes. It includes:
/// - The vote(leader) under which the I/O was submitted
/// - The last log entry that was written (if any logs were appended)
///
/// # Ordering
///
/// Ordered lexicographically as `(vote, last_log_id)`, by the `PartialOrd` of [`Vote`] and of
/// `Option<LogId>`. This is a partial order:
/// - The votes compare first, and a strictly greater vote decides the result whatever the logs: a
///   flush point under a newer vote is greater even when its `last_log_id` is `None`.
/// - Votes of different terms compare by term. Within a term the configured leader id decides: with
///   [`leader_id_std`], two uncommitted votes of different nodes are incomparable, so
///   `partial_cmp()` returns `None`; with [`leader_id_adv`], the higher node id is greater. A
///   committed vote is greater than an uncommitted one with the same or an incomparable leader id.
/// - Under equal votes the log ids decide: `None` is less than any log id, and log ids compare
///   their committed leader id before their index, so a log of term 2 at index 1 is greater than
///   one of term 1 at index 100.
///
/// This lets `wait_until_ge()` wait for a progress milestone. Because a greater vote alone meets
/// the target, a caller that waits for a log entry also checks the vote and the log id it gets
/// back; see [`Raft::watch_log_progress()`].
///
/// [`leader_id_std`]: crate::impls::leader_id_std
/// [`leader_id_adv`]: crate::impls::leader_id_adv
/// [`Raft::watch_log_progress()`]: crate::Raft::watch_log_progress
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
    pub vote: Vote<C::LeaderId>,

    /// The last log entry that was flushed, or `None` if only a vote was saved without appending
    /// logs.
    pub last_log_id: Option<LogIdOf<C>>,
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
    /// Creates a flush point for log I/O submitted under `vote` that flushed up to `last_log_id`.
    pub fn new(vote: Vote<C::LeaderId>, last_log_id: Option<LogIdOf<C>>) -> Self {
        Self { vote, last_log_id }
    }
}
