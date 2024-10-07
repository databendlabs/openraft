use std::fmt;

use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::NodeIdOf;
use crate::vote::ref_vote::RefVote;
use crate::RaftTypeConfig;
use crate::Vote;

/// Represents a non-committed Vote that has **NOT** been granted by a quorum.
///
/// The inner `Vote`'s attribute `committed` is always set to `false`
#[derive(Debug, Clone)]
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

    pub(crate) fn leader_id(&self) -> &LeaderIdOf<C> {
        &self.vote.leader_id
    }

    pub(crate) fn into_vote(self) -> Vote<NodeIdOf<C>> {
        self.vote
    }

    pub(crate) fn as_ref_vote(&self) -> RefVote<'_, C::NodeId> {
        RefVote::new(&self.vote.leader_id, false)
    }
}

impl<C> fmt::Display for NonCommittedVote<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.vote.fmt(f)
    }
}
