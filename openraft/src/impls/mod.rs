//! Default implementations for common Openraft traits.
//!
//! This module provides ready-to-use implementations of Openraft traits and types,
//! making it easy to get started without custom implementations.
//!
//! ## Key Types
//!
//! - [`Entry`] - Default log entry implementation
//! - [`LogId`] - Default log identifier
//! - [`Vote`] - Default vote implementation
//! - [`BasicNode`] - Simple node information with address
//! - [`EmptyNode`] - Minimal node representation (no metadata)
//! - [`OneshotResponder`] - Single-use response channel
//!
//! ## Runtime
//!
//! - [`TokioRuntime`] - Tokio-based async runtime (feature: `tokio-rt`)
//!
//! ## Leader ID Modes
//!
//! - [`leader_id_std::LeaderId`] - Standard Raft: single leader per term
//! - [`leader_id_adv::LeaderId`] - Advanced: multiple leaders per term (reduces conflicts)
//!
//! Most applications can use these implementations directly without customization.

pub use crate::entry::Entry;
pub use crate::node::BasicNode;
pub use crate::node::EmptyNode;
pub use crate::raft::responder::impls::OneshotResponder;
pub use crate::raft::responder::impls::ProgressResponder;
#[cfg(feature = "tokio-rt")]
pub use crate::type_config::async_runtime::tokio_impls::TokioRuntime;

/// LeaderId implementation for advanced mode, allowing multiple leaders per term.
pub mod leader_id_adv {
    pub use crate::vote::leader_id::leader_id_adv::LeaderId;
}

/// LeaderId implementation for standard Raft mode, enforcing single leader per term.
pub mod leader_id_std {
    pub use crate::vote::leader_id::leader_id_std::LeaderId;
}

/// Default implementation of a raft log identity.
pub use crate::log_id::LogId;
/// Default [`RaftVote`] implementation for both standard Raft mode and multi-leader-per-term mode.
///
/// The difference between the two modes is the implementation of [`RaftLeaderId`].
///
/// [`RaftVote`]: crate::vote::raft_vote::RaftVote
/// [`RaftLeaderId`]: crate::vote::RaftLeaderId
pub use crate::vote::Vote;
