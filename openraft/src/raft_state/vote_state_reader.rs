use crate::NodeId;
use crate::Vote;

// TODO: remove it?
/// APIs to get vote.
#[allow(dead_code)]
pub(crate) trait VoteStateReader<NID: NodeId> {
    /// Get a reference to the current vote.
    fn vote_ref(&self) -> &Vote<NID>;
}
