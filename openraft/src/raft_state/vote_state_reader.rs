use crate::NodeId;
use crate::Vote;

/// APIs to get vote.
pub(crate) trait VoteStateReader<NID: NodeId> {
    /// Get a reference to the current vote.
    fn vote_ref(&self) -> &Vote<NID>;
}
