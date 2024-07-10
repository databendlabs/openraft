use std::cmp::Ordering;
use std::fmt;
use std::ops::Deref;

use crate::type_config::alias::NodeIdOf;
use crate::CommittedLeaderId;
use crate::RaftTypeConfig;
use crate::Vote;

/// Represents a committed Vote that has been accepted by a quorum.
///
/// The inner `Vote`'s attribute `committed` is always set to `true`
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
#[derive(PartialOrd)]
pub(crate) struct CommittedVote<C>
where C: RaftTypeConfig
{
    vote: Vote<NodeIdOf<C>>,
}

/// The `CommittedVote` is totally ordered.
///
/// Because:
/// - any two quorums have common elements,
/// - and the `CommittedVote` is accepted by a quorum,
/// - and a `Vote` is granted if it is greater than the old one.
#[allow(clippy::derive_ord_xor_partial_ord)]
impl<C> Ord for CommittedVote<C>
where C: RaftTypeConfig
{
    fn cmp(&self, other: &Self) -> Ordering {
        self.vote.partial_cmp(&other.vote).unwrap()
    }
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
