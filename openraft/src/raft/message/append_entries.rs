use std::fmt;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::raft::AppendEntriesRequest;
use crate::raft::message::log_segment::LogSegment;
use crate::type_config::alias::LogIdOf;
use crate::vote::Leadership;

/// Typed representation of an AppendEntries RPC.
///
/// Decomposes [`AppendEntriesRequest`] into three semantic parts:
/// - [`Leadership`]: the leader's committed vote and last log id
/// - [`LogSegment`]: the contiguous log segment being replicated (anchor + entries)
/// - `leader_commit`: the leader's committed log id
///
/// Converts from [`AppendEntriesRequest`] via [`From`].
pub struct AppendEntries<C: RaftTypeConfig> {
    pub leadership: Leadership<C>,
    pub log_segment: LogSegment<C>,
    pub leader_commit: Option<LogIdOf<C>>,
}

impl<C: RaftTypeConfig> fmt::Display for AppendEntries<C>
where C::Entry: fmt::Display
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}, {}, commit:{}",
            self.leadership,
            self.log_segment,
            self.leader_commit.display()
        )
    }
}

impl<C: RaftTypeConfig> From<AppendEntriesRequest<C>> for AppendEntries<C> {
    fn from(req: AppendEntriesRequest<C>) -> Self {
        AppendEntries {
            leadership: Leadership {
                vote: req.vote,
                // The AppendEntries RPC does not carry the leader's last log id as a
                // separate field, so the last-log-id check is skipped here.
                last_log_id: None,
            },
            log_segment: LogSegment {
                prev_log_id: req.prev_log_id,
                entries: req.entries,
            },
            leader_commit: req.leader_commit,
        }
    }
}
