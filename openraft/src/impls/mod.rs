//! Collection of implementations of usually used traits defined by Openraft

pub use crate::entry::Entry;
pub use crate::node::BasicNode;
pub use crate::node::EmptyNode;
pub use crate::raft::responder::impls::OneshotResponder;
#[cfg(feature = "tokio-rt")]
pub use crate::type_config::async_runtime::tokio_impls::TokioRuntime;

#[cfg(not(feature = "single-term-leader"))]
pub mod leader_id {
    pub use crate::vote::leader_id::leader_id_adv::CommittedLeaderId;
    pub use crate::vote::leader_id::leader_id_adv::LeaderId;
}
#[cfg(feature = "single-term-leader")]
pub mod leader_id {
    pub use crate::vote::leader_id::leader_id_std::CommittedLeaderId;
    pub use crate::vote::leader_id::leader_id_std::LeaderId;
}

pub use leader_id::LeaderId;
