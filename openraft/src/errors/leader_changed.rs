use std::fmt;
use std::fmt::Formatter;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::VoteOf;

/// Error indicating that the stored vote does not show the expected established leader.
///
/// [`RaftLogReader::leader_bounded_stream()`] returns it when no vote is stored, when the stored
/// vote is of another leader or is not committed, or when the vote changed while the entries were
/// read. It reports only what the reader found: `current_vote` need not be greater than the vote
/// of `expected_leader`, and it does not tell which leader the cluster has now.
///
/// [`RaftLogReader::leader_bounded_stream()`]: crate::RaftLogReader::leader_bounded_stream
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct LeaderChanged<C>
where C: RaftTypeConfig
{
    /// The leader that the caller expected the stored vote to show, committed.
    pub expected_leader: LeaderIdOf<C>,

    /// The vote the reader found, or `None` if no vote is stored.
    pub current_vote: Option<VoteOf<C>>,
}

impl<C> fmt::Display for LeaderChanged<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LeaderChanged: from {} to vote {}",
            self.expected_leader,
            self.current_vote.display()
        )
    }
}

impl<C> LeaderChanged<C>
where C: RaftTypeConfig
{
    /// Create a new LeaderChanged error.
    pub fn new(expected_leader: LeaderIdOf<C>, current_vote: Option<VoteOf<C>>) -> Self {
        Self {
            expected_leader,
            current_vote,
        }
    }
}
