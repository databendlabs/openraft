use std::cmp::Ordering;
use std::fmt;

use crate::vote::ref_vote::RefVote;
use crate::CommittedLeaderId;
use crate::RaftTypeConfig;
use crate::Vote;

/// Represents a committed Vote that has been accepted by a quorum.
///
/// The inner `Vote`'s attribute `committed` is always set to `true`
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[derive(PartialOrd)]
pub(crate) struct CommittedVote<C>
where C: RaftTypeConfig
{
    vote: Vote<C>,
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
    pub(crate) fn new(mut vote: Vote<C>) -> Self {
        vote.committed = true;
        Self { vote }
    }

    pub(crate) fn committed_leader_id(&self) -> CommittedLeaderId<C::NodeId> {
        self.vote.leader_id().to_committed()
    }

    pub(crate) fn into_vote(self) -> Vote<C> {
        self.vote
    }

    pub(crate) fn as_ref_vote(&self) -> RefVote<'_, C> {
        RefVote::new(&self.vote.leader_id, true)
    }
}

impl<C> fmt::Display for CommittedVote<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.vote.fmt(f)
    }
}
