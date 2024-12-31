pub mod leader_id_adv;
pub mod leader_id_std;

pub(crate) mod leader_id_cmp;
pub(crate) mod raft_committed_leader_id;
pub(crate) mod raft_leader_id;

#[cfg(feature = "single-term-leader")]
compile_error!(
    r#"`single-term-leader` is removed.
To enable standard Raft mode:
- either add `LeaderId = openraft::impls::leader_id_std::LeaderId` to `declare_raft_types!(YourTypeConfig)` statement,
- or add `type LeaderId: opernaft::impls::leader_id_std::LeaderId` to the `RaftTypeConfig` implementation."#
);
