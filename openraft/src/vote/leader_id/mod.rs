#[cfg(not(feature = "single-term-leader"))]
pub(crate) mod leader_id_adv;
#[cfg(feature = "single-term-leader")]
pub(crate) mod leader_id_std;

#[cfg(not(feature = "single-term-leader"))]
pub use leader_id_adv::CommittedLeaderId;
#[cfg(not(feature = "single-term-leader"))]
pub use leader_id_adv::LeaderId;
#[cfg(feature = "single-term-leader")]
pub use leader_id_std::CommittedLeaderId;
#[cfg(feature = "single-term-leader")]
pub use leader_id_std::LeaderId;

pub(crate) mod raft_committed_leader_id;
pub(crate) mod raft_leader_id;
