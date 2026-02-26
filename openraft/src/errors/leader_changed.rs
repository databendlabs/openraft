use std::fmt;
use std::fmt::Formatter;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::VoteOf;

/// Error indicating that the established leader has changed.
///
/// This error occurs when expecting a specific established leader but finding
/// that the cluster has moved to a different leader or candidate (indicated by a newer vote).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct LeaderChanged<C>
where C: RaftTypeConfig
{
    /// The expected established leader ID.
    pub expected_leader: LeaderIdOf<C>,

    /// The current vote, indicating a new established leader or a candidate.
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
