//! Collection of implementations of usually used traits defined by Openraft

pub use crate::entry::Entry;
pub use crate::node::BasicNode;
pub use crate::node::EmptyNode;
pub use crate::raft::responder::impls::OneshotResponder;
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
