//! Cluster membership configuration and management.
//!
//! This module defines types for managing Raft cluster membership, including voters and learners.
//!
//! ## Key Types
//!
//! - [`Membership`] - Cluster membership configuration (voters and learners)
//! - [`StoredMembership`] - Membership state stored in state machine
//! - [`IntoNodes`] - Trait for converting node sets with metadata
//!
//! ## Overview
//!
//! Membership configuration controls which nodes participate in the cluster:
//! - **Voters**: Contribute to read or write quorums; campaigners may start elections
//! - **Learners**: Receive logs with no campaign, read voter, or write voter role
//!
//! The current [`Membership`] gives every voter all three roles.
//!
//! Membership changes use joint consensus to ensure safety during configuration transitions.
//!
//! See also:
//! - [Dynamic membership guide](crate::docs::cluster_control::dynamic_membership)
//! - [Joint consensus guide](crate::docs::cluster_control::joint_consensus)

mod into_nodes;
#[allow(clippy::module_inception)]
mod membership;
mod membership_impl_quorum_set;
mod membership_impl_raft_membership;
mod stored_membership;

#[cfg(test)]
mod membership_test;
#[cfg(test)]
mod stored_membership_test;

pub use into_nodes::IntoNodes;
pub use membership::Membership;
pub use node_ref::NodeRef;
pub use node_role::NodeRole;
pub use raft_membership::RaftMembership;
pub use stored_membership::StoredMembership;

mod node_ref;
mod node_role;
mod raft_membership;
