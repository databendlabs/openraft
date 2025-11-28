use std::fmt;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::DisplaySlice;
use crate::entry::RaftEntry;
use crate::log_id_range::LogIdRange;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// An RPC sent by a cluster leader to replicate log entries (§5.3), and as a heartbeat (§5.2).
///
/// In Openraft a heartbeat [`AppendEntriesRequest`] message could have `prev_log_id=None` and
/// `entries` empty. Which means: to append nothing at the very beginning position on the Follower,
/// which is always valid. Because `prev_log_id` is used to assert `entries` to be consecutive with
/// the previous log entries, and `prev_log_id=None` is the very beginning position and there are no
/// previous log entries.
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct AppendEntriesRequest<C: RaftTypeConfig> {
    /// The leader's current vote.
    pub vote: VoteOf<C>,

    /// The log id immediately preceding the new entries.
    pub prev_log_id: Option<LogIdOf<C>>,

    /// The new log entries to store.
    ///
    /// This may be empty when the leader is sending heartbeats. Entries
    /// are batched for efficiency.
    pub entries: Vec<C::Entry>,

    /// The leader's committed log id.
    pub leader_commit: Option<LogIdOf<C>>,
}

impl<C: RaftTypeConfig> fmt::Debug for AppendEntriesRequest<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppendEntriesRequest")
            .field("vote", &self.vote)
            .field("prev_log_id", &self.prev_log_id)
            .field("entries", &self.entries)
            .field("leader_commit", &self.leader_commit)
            .finish()
    }
}

impl<C> fmt::Display for AppendEntriesRequest<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "vote={}, prev_log_id={}, leader_commit={}, entries={}",
            self.vote,
            self.prev_log_id.display(),
            self.leader_commit.display(),
            DisplaySlice {
                slice: self.entries.as_slice(),
                max: 5
            }
        )
    }
}

impl<C> AppendEntriesRequest<C>
where C: RaftTypeConfig
{
    /// Returns the last log id in this request.
    ///
    /// This is the log id of the last entry, or `prev_log_id` if entries is empty.
    pub(crate) fn last_log_id(&self) -> Option<LogIdOf<C>> {
        self.entries.last().map(|e| e.log_id()).or(self.prev_log_id.clone())
    }

    /// Returns the log id range covered by this request.
    ///
    /// The range is `(prev_log_id, last_log_id]` where `last_log_id` is the log id
    /// of the last entry, or `prev_log_id` if entries is empty.
    pub(crate) fn log_id_range(&self) -> LogIdRange<C> {
        LogIdRange::new(self.prev_log_id.clone(), self.last_log_id())
    }
}

#[cfg(test)]
mod tests {
    use crate::Vote;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::raft::AppendEntriesRequest;
    use crate::testing::blank_ent;

    fn req(prev: Option<u64>, entries: Vec<u64>) -> AppendEntriesRequest<UTConfig> {
        AppendEntriesRequest {
            vote: Vote::new_committed(1, 1),
            prev_log_id: prev.map(|i| log_id(1, 1, i)),
            entries: entries.into_iter().map(|i| blank_ent::<UTConfig>(1, 1, i)).collect(),
            leader_commit: None,
        }
    }

    #[test]
    fn test_last_log_id() {
        assert_eq!(req(None, vec![]).last_log_id(), None);
        assert_eq!(req(Some(5), vec![]).last_log_id(), Some(log_id(1, 1, 5)));
        assert_eq!(req(None, vec![1, 2]).last_log_id(), Some(log_id(1, 1, 2)));
        assert_eq!(req(Some(5), vec![6, 7]).last_log_id(), Some(log_id(1, 1, 7)));
    }

    #[test]
    fn test_log_id_range() {
        let r = req(None, vec![]).log_id_range();
        assert_eq!((r.prev, r.last), (None, None));

        let r = req(Some(5), vec![]).log_id_range();
        assert_eq!((r.prev, r.last), (Some(log_id(1, 1, 5)), Some(log_id(1, 1, 5))));

        let r = req(None, vec![1, 2]).log_id_range();
        assert_eq!((r.prev, r.last), (None, Some(log_id(1, 1, 2))));

        let r = req(Some(5), vec![6, 7]).log_id_range();
        assert_eq!((r.prev, r.last), (Some(log_id(1, 1, 5)), Some(log_id(1, 1, 7))));
    }
}
