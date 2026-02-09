//! Default implementations for common Openraft traits.
//!
//! This module provides ready-to-use implementations of Openraft traits and types,
//! making it easy to get started without custom implementations.
//!
//! ## Key Types
//!
//! - [`Batch`] - Default batch container implementation
//! - [`Entry`] - Default log entry implementation
//! - [`LogId`] - Default log identifier
//! - [`Vote`] - Default vote implementation
//! - [`BasicNode`] - Simple node information with address
//! - [`EmptyNode`] - Minimal node representation (no metadata)
//! - [`OneshotResponder`] - Single-use response channel
//! - [`BoxedErrorSource`] - Boxed error wrapper for smaller error types
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

mod boxed_error_source;

pub use boxed_error_source::BoxedErrorSource;
#[cfg(feature = "tokio-rt")]
#[deprecated(since = "0.10.0", note = "use `openraft_rt_tokio::TokioRuntime` directly")]
pub use openraft_rt_tokio::TokioRuntime;

// Re-export Batch from base as a default implementation
pub use crate::base::batch::Batch;
pub use crate::entry::Entry;
pub use crate::node::BasicNode;
pub use crate::node::EmptyNode;
pub use crate::raft::responder::impls::OneshotResponder;
pub use crate::raft::responder::impls::ProgressResponder;

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
