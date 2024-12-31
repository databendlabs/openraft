#![allow(clippy::uninlined_format_args)]

use crate::protobuf as pb;
use crate::store::StateMachineData;
use crate::typ::*;

pub mod grpc;
pub mod network;
pub mod store;
#[cfg(test)]
mod test;

mod pb_impl;

pub type NodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig:
        D = pb::SetRequest,
        R = pb::Response,
        LeaderId = pb::LeaderId,
        Node = pb::Node,
        SnapshotData = StateMachineData,
);

pub type LogStore = store::LogStore;
pub type StateMachineStore = store::StateMachineStore;

pub mod protobuf {
    tonic::include_proto!("openraftpb");
}

#[path = "../../utils/declare_types.rs"]
pub mod typ;

impl From<pb::Vote> for Vote {
    fn from(proto_vote: pb::Vote) -> Self {
        let leader_id: LeaderId = proto_vote.leader_id.unwrap();
        if proto_vote.committed {
            Vote::new_committed(leader_id.term, leader_id.node_id)
        } else {
            Vote::new(leader_id.term, leader_id.node_id)
        }
    }
}

impl From<pb::LogId> for LogId {
    fn from(proto_log_id: pb::LogId) -> Self {
        LogId::new(proto_log_id.term, proto_log_id.index)
    }
}

impl From<pb::VoteRequest> for VoteRequest {
    fn from(proto_vote_req: pb::VoteRequest) -> Self {
        let vote: Vote = proto_vote_req.vote.unwrap().into();
        let last_log_id = proto_vote_req.last_log_id.map(|log_id| log_id.into());
        VoteRequest::new(vote, last_log_id)
    }
}

impl From<pb::VoteResponse> for VoteResponse {
    fn from(proto_vote_resp: pb::VoteResponse) -> Self {
        let vote: Vote = proto_vote_resp.vote.unwrap().into();
        let last_log_id = proto_vote_resp.last_log_id.map(|log_id| log_id.into());
        VoteResponse::new(vote, last_log_id, proto_vote_resp.vote_granted)
    }
}

impl From<Vote> for pb::Vote {
    fn from(vote: Vote) -> Self {
        pb::Vote {
            leader_id: Some(pb::LeaderId {
                term: vote.leader_id().term,
                node_id: vote.leader_id().node_id,
            }),
            committed: vote.is_committed(),
        }
    }
}
impl From<LogId> for pb::LogId {
    fn from(log_id: LogId) -> Self {
        pb::LogId {
            term: log_id.leader_id,
            index: log_id.index,
        }
    }
}

impl From<VoteRequest> for pb::VoteRequest {
    fn from(vote_req: VoteRequest) -> Self {
        pb::VoteRequest {
            vote: Some(vote_req.vote.into()),
            last_log_id: vote_req.last_log_id.map(|log_id| log_id.into()),
        }
    }
}

impl From<VoteResponse> for pb::VoteResponse {
    fn from(vote_resp: VoteResponse) -> Self {
        pb::VoteResponse {
            vote: Some(vote_resp.vote.into()),
            vote_granted: vote_resp.vote_granted,
            last_log_id: vote_resp.last_log_id.map(|log_id| log_id.into()),
        }
    }
}
