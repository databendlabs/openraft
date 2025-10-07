//! Cluster membership configuration and management.
//!
//! This module defines types for managing Raft cluster membership, including voters and learners.
//!
//! ## Key Types
//!
//! - [`Membership`] - Cluster membership configuration (voters and learners)
//! - [`EffectiveMembership`] - Currently active membership, including joint consensus state
//! - [`StoredMembership`] - Membership state stored in state machine
//! - [`IntoNodes`] - Trait for converting node sets with metadata
//!
//! ## Overview
//!
//! Membership configuration controls which nodes participate in the cluster:
//! - **Voters**: Participate in elections and voting, can become leader
//! - **Learners**: Receive log replication but don't vote
//!
//! Membership changes use joint consensus to ensure safety during configuration transitions.
//!
//! See also:
//! - [Dynamic membership guide](crate::docs::cluster_control::dynamic_membership)
//! - [Joint consensus guide](crate::docs::cluster_control::joint_consensus)

mod effective_membership;
mod into_nodes;
#[allow(clippy::module_inception)]
mod membership;
mod stored_membership;

#[cfg(feature = "bench")]
#[cfg(test)]
mod bench;

#[cfg(test)]
mod effective_membership_test;
#[cfg(test)]
mod membership_test;

pub use effective_membership::EffectiveMembership;
pub use into_nodes::IntoNodes;
pub use membership::Membership;
pub use stored_membership::StoredMembership;
