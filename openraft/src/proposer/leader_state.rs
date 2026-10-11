use std::sync::Arc;

use crate::Membership;
use crate::proposer::Candidate;
use crate::proposer::Leader;
use crate::quorum::ReadOrBoth;
use crate::type_config::alias::NodeIdOf;
use crate::type_config::alias::NodeOf;

/// The write quorum set: a leader commits log entries and keeps its lease with it.
pub(crate) type WriteQuorumSet<C> = Arc<Membership<NodeIdOf<C>, NodeOf<C>>>;

/// The read quorum set: every vote set that wins an election is one of its quorums.
pub(crate) type ReadQuorumSet<C> = Arc<Membership<NodeIdOf<C>, NodeOf<C>>>;

/// The quorum set type used by `Candidate`: the election rule for the leader ID mode.
///
/// [Election quorum](crate::docs::protocol::election_quorum) explains why.
pub(crate) type ElectionQuorumSet<C> = ReadOrBoth<ReadQuorumSet<C>, WriteQuorumSet<C>>;

pub(crate) type LeaderState<C> = Option<Box<Leader<C, WriteQuorumSet<C>>>>;
pub(crate) type CandidateState<C> = Option<Candidate<C, ElectionQuorumSet<C>>>;
