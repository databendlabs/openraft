use std::sync::Arc;

use openraft::raft::AppendEntriesRequest;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::VoteRequest;
use tonic::Request;
use tonic::Response;

use crate::protobuf::internal_service_server::InternalService;
use crate::protobuf::RaftReply;
use crate::protobuf::RaftRequest;
use crate::protobuf::SnapshotChunkRequest;
use crate::StringWrapper;
use crate::TypeConfig;

pub struct RaftInternalService {
    raft: Arc<openraft::Raft<TypeConfig>>,
}

impl RaftInternalService {
    pub async fn new(raft: Arc<openraft::Raft<TypeConfig>>) -> Result<Self, openraft::error::RaftError<TypeConfig>> {
        Ok(Self { raft })
    }
}

#[tonic::async_trait]
impl InternalService for RaftInternalService {
    async fn vote(&self, request: Request<RaftRequest>) -> Result<Response<RaftReply>, tonic::Status> {
        let vote_request = request.into_inner();
        let response = self.raft.vote(VoteRequest::from(StringWrapper(vote_request.data))).await;
        Ok(Response::new(RaftReply::from(response)))
    }

    async fn append_entries(&self, request: Request<RaftRequest>) -> Result<Response<RaftReply>, tonic::Status> {
        let append_entries_request = request.into_inner();
        let response = self
            .raft
            .append_entries(AppendEntriesRequest::from(StringWrapper(append_entries_request.data)))
            .await;
        Ok(Response::new(RaftReply::from(response)))
    }

    async fn install_snapshot(
        &self,
        request: Request<SnapshotChunkRequest>,
    ) -> Result<Response<RaftReply>, tonic::Status> {
        let install_snapshot_request = request.into_inner();
        let response = self
            .raft
            .install_snapshot(InstallSnapshotRequest::from(StringWrapper(
                install_snapshot_request.rpc_meta,
            )))
            .await;
        Ok(Response::new(RaftReply::from(response)))
    }
}
