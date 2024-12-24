#![allow(clippy::uninlined_format_args)]

use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::LeaderId;
use openraft::LogId;

use crate::protobuf::Node;
use crate::protobuf::Response;
use crate::protobuf::SetRequest;
use crate::store::StateMachineData;

pub mod grpc;
pub mod network;
pub mod store;
#[cfg(test)]
mod test;

pub type NodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig:
        D = SetRequest,
        R = Response,
        Node = Node,
        SnapshotData = StateMachineData,
);

pub type LogStore = store::LogStore;
pub type StateMachineStore = store::StateMachineStore;
pub type Raft = openraft::Raft<TypeConfig>;

pub mod protobuf {
    tonic::include_proto!("openraftpb");
}

pub mod typ {

    use crate::TypeConfig;

    pub type Vote = openraft::Vote<TypeConfig>;
    pub type SnapshotMeta = openraft::SnapshotMeta<TypeConfig>;
    pub type SnapshotData = <TypeConfig as openraft::RaftTypeConfig>::SnapshotData;
    pub type Snapshot = openraft::Snapshot<TypeConfig>;

    pub type RaftError<E = openraft::error::Infallible> = openraft::error::RaftError<TypeConfig, E>;
    pub type RPCError = openraft::error::RPCError<TypeConfig>;
    pub type StreamingError = openraft::error::StreamingError<TypeConfig>;

    pub type ClientWriteError = openraft::error::ClientWriteError<TypeConfig>;
    pub type CheckIsLeaderError = openraft::error::CheckIsLeaderError<TypeConfig>;
    pub type ForwardToLeader = openraft::error::ForwardToLeader<TypeConfig>;
    pub type InitializeError = openraft::error::InitializeError<TypeConfig>;

    pub type ClientWriteResponse = openraft::raft::ClientWriteResponse<TypeConfig>;
}

impl From<protobuf::LeaderId> for LeaderId<NodeId> {
    fn from(proto_leader_id: protobuf::LeaderId) -> Self {
        LeaderId::new(proto_leader_id.term, proto_leader_id.node_id)
    }
}

impl From<protobuf::Vote> for typ::Vote {
    fn from(proto_vote: protobuf::Vote) -> Self {
        let leader_id: LeaderId<NodeId> = proto_vote.leader_id.unwrap().into();
        if proto_vote.committed {
            typ::Vote::new_committed(leader_id.term, leader_id.node_id)
        } else {
            typ::Vote::new(leader_id.term, leader_id.node_id)
        }
    }
}

impl From<protobuf::LogId> for LogId<TypeConfig> {
    fn from(proto_log_id: protobuf::LogId) -> Self {
        let leader_id: LeaderId<NodeId> = proto_log_id.leader_id.unwrap().into();
        LogId::new(leader_id, proto_log_id.index)
    }
}

impl From<protobuf::VoteRequest> for VoteRequest<TypeConfig> {
    fn from(proto_vote_req: protobuf::VoteRequest) -> Self {
        let vote: typ::Vote = proto_vote_req.vote.unwrap().into();
        let last_log_id = proto_vote_req.last_log_id.map(|log_id| log_id.into());
        VoteRequest::new(vote, last_log_id)
    }
}

impl From<protobuf::VoteResponse> for VoteResponse<TypeConfig> {
    fn from(proto_vote_resp: protobuf::VoteResponse) -> Self {
        let vote: typ::Vote = proto_vote_resp.vote.unwrap().into();
        let last_log_id = proto_vote_resp.last_log_id.map(|log_id| log_id.into());
        VoteResponse::new(vote, last_log_id, proto_vote_resp.vote_granted)
    }
}

impl From<LeaderId<NodeId>> for protobuf::LeaderId {
    fn from(leader_id: LeaderId<NodeId>) -> Self {
        protobuf::LeaderId {
            term: leader_id.term,
            node_id: leader_id.node_id,
        }
    }
}

impl From<typ::Vote> for protobuf::Vote {
    fn from(vote: typ::Vote) -> Self {
        protobuf::Vote {
            leader_id: Some(protobuf::LeaderId {
                term: vote.leader_id().term,
                node_id: vote.leader_id().node_id,
            }),
            committed: vote.is_committed(),
        }
    }
}
impl From<LogId<TypeConfig>> for protobuf::LogId {
    fn from(log_id: LogId<TypeConfig>) -> Self {
        protobuf::LogId {
            index: log_id.index,
            leader_id: Some(log_id.leader_id.into()),
        }
    }
}

impl From<VoteRequest<TypeConfig>> for protobuf::VoteRequest {
    fn from(vote_req: VoteRequest<TypeConfig>) -> Self {
        protobuf::VoteRequest {
            vote: Some(vote_req.vote.into()),
            last_log_id: vote_req.last_log_id.map(|log_id| log_id.into()),
        }
    }
}

impl From<VoteResponse<TypeConfig>> for protobuf::VoteResponse {
    fn from(vote_resp: VoteResponse<TypeConfig>) -> Self {
        protobuf::VoteResponse {
            vote: Some(vote_resp.vote.into()),
            vote_granted: vote_resp.vote_granted,
            last_log_id: vote_resp.last_log_id.map(|log_id| log_id.into()),
        }
    }
}
