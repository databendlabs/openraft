//! Cluster membership configuration and management.
//!
//! This module defines types for managing Raft cluster membership, including voters and learners.
//!
//! ## Key Types
//!
//! - [`Membership`] - Cluster membership configuration (voters and learners)
//! - [`StoredMembership`] - Membership state stored in state machine
//! - [`IntoNodes`] - Trait for converting node sets with metadata
//! - [`QuorumSet`], [`QuorumIntersection`] and [`QuorumBridge`] - Traits from the quorum-set crate
//!   that the quorum set of a [`RaftMembership`] implements
//! - [`ThresholdQuorumSet`] - Quorum rule that accepts any `quorum_size` of its IDs
//! - [`Joint`] - Quorum rule that accepts an ID set only when each of its rules accepts it
//! - [`ThresholdMembership`] - Membership with a configurable write quorum size and the minimum
//!   intersecting read quorum size
//! - [`ThresholdChange`] - Change to the voters, nodes, or sizes of a [`ThresholdMembership`]
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
mod threshold;
mod unproven_read_write_intersection;

#[cfg(test)]
mod membership_test;
#[cfg(test)]
mod stored_membership_test;

pub use into_nodes::IntoNodes;
pub use membership::Membership;
pub use node_ref::NodeRef;
pub use node_role::NodeRole;
use openraft_macros::since;
#[since(version = "0.10.0")]
pub use quorum_set::QuorumBridge;
#[since(version = "0.10.0")]
pub use quorum_set::QuorumIntersection;
#[since(version = "0.10.0")]
pub use quorum_set::QuorumSet;
pub use raft_membership::RaftMembership;
pub use raft_membership_edit::RaftMembershipEdit;
pub(crate) use raft_membership_ext::RaftMembershipExt;
pub use stored_membership::StoredMembership;
pub use threshold::InvalidThresholds;
pub use threshold::ThresholdChange;
pub use threshold::ThresholdMembership;
pub use threshold::ThresholdMembershipError;
#[since(version = "0.10.0")]
pub use unproven_read_write_intersection::UnprovenReadWriteIntersection;

pub use crate::quorum::Joint;
pub use crate::quorum::ThresholdQuorumSet;

mod node_ref;
mod node_role;
mod raft_membership;
mod raft_membership_edit;
mod raft_membership_ext;
