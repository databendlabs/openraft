#![allow(clippy::uninlined_format_args)]

pub mod app;
pub mod network;
pub mod raft;
pub mod store;

#[path = "../../utils/declare_types.rs"]
pub mod typ;

#[cfg(test)]
mod test_store;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig:
        D = raft::SetRequest,
        R = raft::Response,
        LeaderId = raft::LeaderId,
        Vote = raft::Vote,
        Entry = raft::Entry,
        Node = raft::Node,
        SnapshotData = Vec<u8>,
);

pub type NodeId = u64;
pub type LogStore = store::LogStore;
pub type StateMachineStore = store::StateMachineStore;
