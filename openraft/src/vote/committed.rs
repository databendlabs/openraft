use std::fmt;
use std::ops::Deref;

use crate::type_config::alias::NodeIdOf;
use crate::CommittedLeaderId;
use crate::RaftTypeConfig;
use crate::Vote;

/// Represents a committed Vote that has been accepted by a quorum.
///
/// The inner `Vote`'s attribute `committed` is always set to `true`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd)]
pub(crate) struct CommittedVote<C>
where C: RaftTypeConfig
{
    vote: Vote<NodeIdOf<C>>,
}

impl<C> CommittedVote<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(mut vote: Vote<NodeIdOf<C>>) -> Self {
        vote.committed = true;
        Self { vote }
    }
    pub(crate) fn committed_leader_id(&self) -> CommittedLeaderId<C::NodeId> {
        self.leader_id().to_committed()
    }
}

impl<C> Deref for CommittedVote<C>
where C: RaftTypeConfig
{
    type Target = Vote<NodeIdOf<C>>;

    fn deref(&self) -> &Self::Target {
        &self.vote
    }
}

impl<C> fmt::Display for CommittedVote<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.vote.fmt(f)
    }
}
