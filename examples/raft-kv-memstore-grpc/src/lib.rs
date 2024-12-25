#![allow(clippy::uninlined_format_args)]

use crate::protobuf::Node;
use crate::protobuf::Response;
use crate::protobuf::SetRequest;
use crate::store::StateMachineData;
use crate::typ::*;

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

pub mod protobuf {
    tonic::include_proto!("openraftpb");
}

#[path = "../../utils/declare_types.rs"]
pub mod typ;

impl From<protobuf::LeaderId> for LeaderId {
    fn from(proto_leader_id: protobuf::LeaderId) -> Self {
        LeaderId::new(proto_leader_id.term, proto_leader_id.node_id)
    }
}

impl From<protobuf::Vote> for typ::Vote {
    fn from(proto_vote: protobuf::Vote) -> Self {
        let leader_id: LeaderId = proto_vote.leader_id.unwrap().into();
        if proto_vote.committed {
            typ::Vote::new_committed(leader_id.term, leader_id.node_id)
        } else {
            typ::Vote::new(leader_id.term, leader_id.node_id)
        }
    }
}

impl From<protobuf::LogId> for LogId {
    fn from(proto_log_id: protobuf::LogId) -> Self {
        let leader_id: LeaderId = proto_log_id.leader_id.unwrap().into();
        LogId::new(leader_id, proto_log_id.index)
    }
}

impl From<protobuf::VoteRequest> for VoteRequest {
    fn from(proto_vote_req: protobuf::VoteRequest) -> Self {
        let vote: typ::Vote = proto_vote_req.vote.unwrap().into();
        let last_log_id = proto_vote_req.last_log_id.map(|log_id| log_id.into());
        VoteRequest::new(vote, last_log_id)
    }
}

impl From<protobuf::VoteResponse> for VoteResponse {
    fn from(proto_vote_resp: protobuf::VoteResponse) -> Self {
        let vote: typ::Vote = proto_vote_resp.vote.unwrap().into();
        let last_log_id = proto_vote_resp.last_log_id.map(|log_id| log_id.into());
        VoteResponse::new(vote, last_log_id, proto_vote_resp.vote_granted)
    }
}

impl From<LeaderId> for protobuf::LeaderId {
    fn from(leader_id: LeaderId) -> Self {
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
impl From<LogId> for protobuf::LogId {
    fn from(log_id: LogId) -> Self {
        protobuf::LogId {
            index: log_id.index,
            leader_id: Some(log_id.leader_id.into()),
        }
    }
}

impl From<VoteRequest> for protobuf::VoteRequest {
    fn from(vote_req: VoteRequest) -> Self {
        protobuf::VoteRequest {
            vote: Some(vote_req.vote.into()),
            last_log_id: vote_req.last_log_id.map(|log_id| log_id.into()),
        }
    }
}

impl From<VoteResponse> for protobuf::VoteResponse {
    fn from(vote_resp: VoteResponse) -> Self {
        protobuf::VoteResponse {
            vote: Some(vote_resp.vote.into()),
            vote_granted: vote_resp.vote_granted,
            last_log_id: vote_resp.last_log_id.map(|log_id| log_id.into()),
        }
    }
}
