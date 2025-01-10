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
        Vote = pb::Vote,
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

impl From<pb::LogId> for LogId {
    fn from(proto_log_id: pb::LogId) -> Self {
        LogId::new(proto_log_id.term, proto_log_id.index)
    }
}

impl From<pb::VoteRequest> for VoteRequest {
    fn from(proto_vote_req: pb::VoteRequest) -> Self {
        let vote = proto_vote_req.vote.unwrap();
        let last_log_id = proto_vote_req.last_log_id.map(|log_id| log_id.into());
        VoteRequest::new(vote, last_log_id)
    }
}

impl From<pb::VoteResponse> for VoteResponse {
    fn from(proto_vote_resp: pb::VoteResponse) -> Self {
        let vote = proto_vote_resp.vote.unwrap();
        let last_log_id = proto_vote_resp.last_log_id.map(|log_id| log_id.into());
        VoteResponse::new(vote, last_log_id, proto_vote_resp.vote_granted)
    }
}

impl From<LogId> for pb::LogId {
    fn from(log_id: LogId) -> Self {
        pb::LogId {
            term: log_id.leader_id,
            index: log_id.index(),
        }
    }
}

impl From<VoteRequest> for pb::VoteRequest {
    fn from(vote_req: VoteRequest) -> Self {
        pb::VoteRequest {
            vote: Some(vote_req.vote),
            last_log_id: vote_req.last_log_id.map(|log_id| log_id.into()),
        }
    }
}

impl From<VoteResponse> for pb::VoteResponse {
    fn from(vote_resp: VoteResponse) -> Self {
        pb::VoteResponse {
            vote: Some(vote_resp.vote),
            vote_granted: vote_resp.vote_granted,
            last_log_id: vote_resp.last_log_id.map(|log_id| log_id.into()),
        }
    }
}
