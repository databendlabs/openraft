use std::fmt;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::DisplaySlice;
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

/// The response to an `AppendEntriesRequest`.
///
/// [`RaftNetwork::append_entries`] returns this type only when received an RPC reply.
/// Otherwise, it should return [`RPCError`].
///
/// [`RPCError`]: crate::error::RPCError
/// [`RaftNetwork::append_entries`]: crate::network::RaftNetwork::append_entries
#[derive(Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum AppendEntriesResponse<C: RaftTypeConfig> {
    /// Successfully replicated all log entries to the target node.
    Success,

    /// Successfully sent the first portion of log entries.
    ///
    /// [`RaftNetwork::append_entries`] can return a partial success.
    /// For example, it tries to send log entries `[1-2..3-10]`, the application is allowed to send
    /// just `[1-2..1-3]` and return `PartialSuccess(1-3)`
    ///
    /// ### Caution
    ///
    /// The returned matching log id must be **greater than or equal to** the first log
    /// id([`AppendEntriesRequest::prev_log_id`]) of the entries to send. If no RPC reply is
    /// received, [`RaftNetwork::append_entries`] must return an [`RPCError`] to inform
    /// Openraft that the first log id([`AppendEntriesRequest::prev_log_id`]) may not match on
    /// the remote target node.
    ///
    /// [`RPCError`]: crate::error::RPCError
    /// [`RaftNetwork::append_entries`]: crate::network::RaftNetwork::append_entries
    PartialSuccess(Option<LogIdOf<C>>),

    /// The first log id([`AppendEntriesRequest::prev_log_id`]) of the entries to send does not
    /// match on the remote target node.
    Conflict,

    /// Seen a vote `v` that does not hold `mine_vote >= v`.
    /// And a leader's vote(committed vote) must be total order with other votes.
    /// Therefore, it has to be a higher vote: `mine_vote < v`
    HigherVote(VoteOf<C>),
}

impl<C> AppendEntriesResponse<C>
where C: RaftTypeConfig
{
    /// Returns true if the response indicates a successful replication.
    pub fn is_success(&self) -> bool {
        matches!(*self, AppendEntriesResponse::Success)
    }

    /// Returns true if the response indicates a log conflict.
    pub fn is_conflict(&self) -> bool {
        matches!(*self, AppendEntriesResponse::Conflict)
    }
}

impl<C> fmt::Display for AppendEntriesResponse<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppendEntriesResponse::Success => write!(f, "Success"),
            AppendEntriesResponse::PartialSuccess(m) => {
                write!(f, "PartialSuccess({})", m.display())
            }
            AppendEntriesResponse::HigherVote(vote) => write!(f, "Higher vote, {}", vote),
            AppendEntriesResponse::Conflict => write!(f, "Conflict"),
        }
    }
}
