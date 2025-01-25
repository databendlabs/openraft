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
        Entry = pb::Entry,
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

impl From<pb::AppendEntriesRequest> for AppendEntriesRequest {
    fn from(proto_req: pb::AppendEntriesRequest) -> Self {
        AppendEntriesRequest {
            vote: proto_req.vote.unwrap(),
            prev_log_id: proto_req.prev_log_id.map(|log_id| log_id.into()),
            entries: proto_req.entries,
            leader_commit: proto_req.leader_commit.map(|log_id| log_id.into()),
        }
    }
}

impl From<AppendEntriesRequest> for pb::AppendEntriesRequest {
    fn from(value: AppendEntriesRequest) -> Self {
        pb::AppendEntriesRequest {
            vote: Some(value.vote),
            prev_log_id: value.prev_log_id.map(|log_id| log_id.into()),
            entries: value.entries,
            leader_commit: value.leader_commit.map(|log_id| log_id.into()),
        }
    }
}

impl From<pb::AppendEntriesResponse> for AppendEntriesResponse {
    fn from(r: pb::AppendEntriesResponse) -> Self {
        if let Some(higher) = r.rejected_by {
            return AppendEntriesResponse::HigherVote(higher);
        }

        if r.conflict {
            return AppendEntriesResponse::Conflict;
        }

        if let Some(log_id) = r.last_log_id {
            AppendEntriesResponse::PartialSuccess(Some(log_id.into()))
        } else {
            AppendEntriesResponse::Success
        }
    }
}

impl From<AppendEntriesResponse> for pb::AppendEntriesResponse {
    fn from(r: AppendEntriesResponse) -> Self {
        match r {
            AppendEntriesResponse::Success => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: None,
            },
            AppendEntriesResponse::PartialSuccess(p) => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: false,
                last_log_id: p.map(|log_id| log_id.into()),
            },
            AppendEntriesResponse::Conflict => pb::AppendEntriesResponse {
                rejected_by: None,
                conflict: true,
                last_log_id: None,
            },
            AppendEntriesResponse::HigherVote(v) => pb::AppendEntriesResponse {
                rejected_by: Some(v),
                conflict: false,
                last_log_id: None,
            },
        }
    }
}

impl From<LogId> for pb::LogId {
    fn from(log_id: LogId) -> Self {
        pb::LogId {
            term: *log_id.leader_id(),
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
