//! Defines election-related types.

pub(crate) mod committed;
pub(crate) mod leader_id;
pub(crate) mod non_committed;
pub(crate) mod raft_term;
pub(crate) mod raft_vote;
pub(crate) mod ref_vote;
#[allow(clippy::module_inception)]
mod vote;
pub(crate) mod vote_status;

pub use leader_id::raft_committed_leader_id::RaftCommittedLeaderId;
pub use leader_id::raft_leader_id::RaftLeaderId;
pub use leader_id::raft_leader_id::RaftLeaderIdExt;
pub use raft_term::RaftTerm;
pub use raft_vote::RaftVote;

pub use self::leader_id::leader_id_adv;
pub use self::leader_id::leader_id_cmp::LeaderIdCompare;
pub use self::leader_id::leader_id_std;
pub use self::vote::Vote;
