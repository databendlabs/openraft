use std::fmt;
use std::ops::Deref;

use crate::type_config::alias::NodeIdOf;
use crate::RaftTypeConfig;
use crate::Vote;

/// Represents a committed Vote that has been accepted by a quorum.
///
/// The inner `Vote`'s attribute `committed` is always set to `true`
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
#[derive(PartialOrd)]
pub(crate) struct NonCommittedVote<C>
where C: RaftTypeConfig
{
    vote: Vote<NodeIdOf<C>>,
}

impl<C> NonCommittedVote<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(vote: Vote<NodeIdOf<C>>) -> Self {
        debug_assert!(!vote.committed);
        Self { vote }
    }
}

impl<C> Deref for NonCommittedVote<C>
where C: RaftTypeConfig
{
    type Target = Vote<NodeIdOf<C>>;

    fn deref(&self) -> &Self::Target {
        &self.vote
    }
}

impl<C> fmt::Display for NonCommittedVote<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.vote.fmt(f)
    }
}
