//! A proposer includes the Candidate(phase-1) state and Leader(phase-2) state.

pub(crate) mod candidate;
pub(crate) mod leader;
pub(crate) mod leader_state;

pub(crate) use candidate::Candidate;
pub(crate) use leader::Leader;
pub(crate) use leader_state::CandidateState;
pub(crate) use leader_state::LeaderQuorumSet;
pub(crate) use leader_state::LeaderState;
