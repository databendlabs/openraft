use std::fmt;
use std::sync::Arc;
use std::sync::OnceLock;

use futures::future::join_all;
use openraft::raft::AppendError;
use raft_kv_memstore_grpc::protobuf as raftpb;
use raft_kv_memstore_grpc::typ::Snapshot;
use raft_kv_memstore_grpc::typ::SnapshotMeta;
use raft_kv_memstore_grpc::typ::StoredMembership;
use tonic::Request;
use tonic::Response;
use tonic::Status;

use crate::Raft;
use crate::partition::Partition;
use crate::protobuf as pb;
use crate::protobuf::group_append_result::Result as BatchResult;
use crate::protobuf::group_raft_server::GroupRaft;
use crate::rejoins::Rejoins;

/// Passes each Raft request from a peer node to this node's member of its group.
pub(crate) struct RaftService {
    /// This node's member of each group, by group id, set once at setup.
    groups: Arc<OnceLock<Vec<Raft>>>,

    /// Learns of every AppendEntries request that a member accepts.
    rejoins: Rejoins,

    /// Holds the requests while this node is cut off from its peers.
    partition: Partition,
}

impl RaftService {
    pub(crate) fn new(groups: Arc<OnceLock<Vec<Raft>>>, rejoins: Rejoins, partition: Partition) -> Self {
        Self {
            groups,
            rejoins,
            partition,
        }
    }

    fn group(&self, group_id: u64) -> Result<&Raft, Status> {
        let groups = self.groups.get().ok_or_else(|| Status::unavailable("the groups are not created yet"))?;
        let group = usize::try_from(group_id).ok().and_then(|index| groups.get(index));
        group.ok_or_else(|| Status::not_found(format!("no group {group_id}")))
    }

    /// Pass `request` to this node's member of its group, and return the member's reply.
    async fn append_to_group(&self, request: pb::GroupAppendRequest) -> Result<raftpb::AppendEntriesResponse, Status> {
        let pb::GroupAppendRequest { group_id, request } = request;
        let request = request.ok_or_else(|| Status::invalid_argument("no AppendEntries request"))?;

        let result = self.group(group_id)?.append_entries(request.into()).await.map_err(internal)?;
        // A member that rejects the sender's vote does not accept it as the leader.
        if !matches!(result, Err(AppendError::HigherVote(_))) {
            self.rejoins.accepted(group_id);
        }
        Ok(result.into())
    }
}

#[tonic::async_trait]
impl GroupRaft for RaftService {
    async fn append(
        &self,
        request: Request<pb::GroupAppendRequest>,
    ) -> Result<Response<raftpb::AppendEntriesResponse>, Status> {
        self.partition.hold().await;
        let response = self.append_to_group(request.into_inner()).await?;
        Ok(Response::new(response))
    }

    async fn append_batch(
        &self,
        request: Request<pb::GroupAppendBatch>,
    ) -> Result<Response<pb::GroupAppendBatchResponse>, Status> {
        let requests = request.into_inner().requests;

        self.partition.hold().await;
        // The members handle the requests at the same time, as they would handle separate calls.
        let results = join_all(requests.into_iter().map(|request| self.append_to_group(request))).await;

        let result = |result: Result<raftpb::AppendEntriesResponse, Status>| {
            let result = match result {
                Ok(response) => BatchResult::Response(response),
                Err(status) => BatchResult::Error(pb::GroupAppendError {
                    code: status.code() as i32,
                    message: status.message().to_string(),
                }),
            };
            pb::GroupAppendResult { result: Some(result) }
        };
        Ok(Response::new(pb::GroupAppendBatchResponse {
            results: results.into_iter().map(result).collect(),
        }))
    }

    async fn vote(&self, request: Request<pb::GroupVoteRequest>) -> Result<Response<raftpb::VoteResponse>, Status> {
        let pb::GroupVoteRequest { group_id, request } = request.into_inner();
        let request = request.ok_or_else(|| Status::invalid_argument("no Vote request"))?;

        self.partition.hold().await;
        let response = self.group(group_id)?.vote(request.into()).await.map_err(internal)?;
        Ok(Response::new(response.into()))
    }

    async fn pre_vote(&self, request: Request<pb::GroupVoteRequest>) -> Result<Response<raftpb::VoteResponse>, Status> {
        let pb::GroupVoteRequest { group_id, request } = request.into_inner();
        let request = request.ok_or_else(|| Status::invalid_argument("no PreVote request"))?;

        self.partition.hold().await;
        let response = self.group(group_id)?.pre_vote(request.into()).await.map_err(internal)?;
        Ok(Response::new(response.into()))
    }

    async fn snapshot(
        &self,
        request: Request<pb::GroupSnapshotRequest>,
    ) -> Result<Response<raftpb::SnapshotResponse>, Status> {
        let pb::GroupSnapshotRequest { group_id, meta, data } = request.into_inner();
        self.partition.hold().await;
        let meta = meta.ok_or_else(|| Status::invalid_argument("no snapshot meta"))?;
        let vote = meta.vote.ok_or_else(|| Status::invalid_argument("no vote in the snapshot meta"))?;
        let membership =
            meta.last_membership.ok_or_else(|| Status::invalid_argument("no membership in the snapshot meta"))?;

        let snapshot = Snapshot {
            meta: SnapshotMeta {
                last_log_id: meta.last_log_id.map(Into::into),
                last_membership: StoredMembership::new(meta.last_membership_log_id.map(Into::into), membership.into()),
            },
            snapshot: data,
        };
        let response = self.group(group_id)?.install_full_snapshot(vote, snapshot).await.map_err(internal)?;
        Ok(Response::new(raftpb::SnapshotResponse {
            vote: Some(response.vote),
        }))
    }
}

/// Report an error of this node's member, which has stopped, to the peer as an internal error.
fn internal<E>(error: E) -> Status
where E: fmt::Display {
    Status::internal(error.to_string())
}
