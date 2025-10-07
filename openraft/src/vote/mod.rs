//! Election and voting types.
//!
//! This module defines types for Raft leader election, voting, and term management.
//!
//! ## Key Types
//!
//! - [`Vote`] - A vote for a leader candidate, including term and node ID
//! - [`RaftTerm`] - Raft term number for tracking leadership epochs
//! - [`RaftLeaderId`] - Identifier for a leader (term + node ID)
//! - [`RaftCommittedLeaderId`] - Leader ID that has been committed
//!
//! ## Overview
//!
//! Voting is central to Raft's leader election mechanism:
//! - Each node maintains its current vote
//! - Votes include term and candidate node ID
//! - A candidate must receive votes from a quorum to become leader
//!
//! ## Leader ID Modes
//!
//! Openraft supports two leader ID modes:
//! - [`leader_id_std`] - Standard Raft: one leader per term
//! - [`leader_id_adv`] - Advanced mode: multiple leaders per term (reduces election conflicts)
//!
//! See the [leader ID documentation](crate::docs::data::leader_id) for details.

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
