use std::sync::Arc;

use crate::Membership;
use crate::proposer::Candidate;
use crate::proposer::Leader;
use crate::type_config::alias::NodeIdOf;
use crate::type_config::alias::NodeOf;

/// The quorum set type used by `Leader`.
pub(crate) type LeaderQuorumSet<C> = Arc<Membership<NodeIdOf<C>, NodeOf<C>>>;

pub(crate) type LeaderState<C> = Option<Box<Leader<C, LeaderQuorumSet<C>>>>;
pub(crate) type CandidateState<C> = Option<Candidate<C, LeaderQuorumSet<C>>>;
