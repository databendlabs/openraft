use crate::proposer::Candidate;
use crate::proposer::Leader;
use crate::quorum::Joint;
use crate::type_config::alias::NodeIdOf;

/// The quorum set type used by `Leader`.
pub(crate) type LeaderQuorumSet<NID> = Joint<NID, Vec<NID>, Vec<Vec<NID>>>;

pub(crate) type LeaderState<C> = Option<Box<Leader<C, LeaderQuorumSet<NodeIdOf<C>>>>>;
pub(crate) type CandidateState<C> = Option<Candidate<C, LeaderQuorumSet<NodeIdOf<C>>>>;
