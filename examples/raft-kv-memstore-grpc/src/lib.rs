#![allow(clippy::uninlined_format_args)]

use std::fmt::Display;

use crate::store::Request;
use crate::store::Response;

pub mod grpc;
pub mod network;
pub mod store;
#[cfg(test)]
mod test;

pub type NodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig:
        D = Request,
        R = Response,
        Node = Node,
);

pub type LogStore = store::LogStore;
pub type StateMachineStore = store::StateMachineStore;
pub type Raft = openraft::Raft<TypeConfig>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct Node {
    pub node_id: u64,
    pub rpc_addr: String,
}

impl Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Node {{ rpc_addr: {}, node_id: {} }}", self.rpc_addr, self.node_id)
    }
}

pub mod protobuf {
    tonic::include_proto!("openraftpb");
}

pub mod typ {

    use crate::TypeConfig;

    pub type RaftError<E = openraft::error::Infallible> = openraft::error::RaftError<TypeConfig, E>;
    pub type RPCError<E = openraft::error::Infallible> = openraft::error::RPCError<TypeConfig, RaftError<E>>;

    pub type ClientWriteError = openraft::error::ClientWriteError<TypeConfig>;
    pub type CheckIsLeaderError = openraft::error::CheckIsLeaderError<TypeConfig>;
    pub type ForwardToLeader = openraft::error::ForwardToLeader<TypeConfig>;
    pub type InitializeError = openraft::error::InitializeError<TypeConfig>;

    pub type ClientWriteResponse = openraft::raft::ClientWriteResponse<TypeConfig>;
}
