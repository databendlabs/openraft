use std::fmt;
use std::sync::Arc;
use std::sync::OnceLock;

use raft_kv_memstore_grpc::protobuf as raftpb;
use raft_kv_memstore_grpc::typ::Snapshot;
use raft_kv_memstore_grpc::typ::SnapshotMeta;
use raft_kv_memstore_grpc::typ::StoredMembership;
use tonic::Request;
use tonic::Response;
use tonic::Status;

use crate::Raft;
use crate::protobuf as pb;
use crate::protobuf::group_raft_server::GroupRaft;

/// Passes each Raft request from a peer node to this node's member of its group.
pub(crate) struct RaftService {
    /// This node's member of each group, by group id, set once at setup.
    groups: Arc<OnceLock<Vec<Raft>>>,
}

impl RaftService {
    pub(crate) fn new(groups: Arc<OnceLock<Vec<Raft>>>) -> Self {
        Self { groups }
    }

    fn group(&self, group_id: u64) -> Result<&Raft, Status> {
        let groups = self.groups.get().ok_or_else(|| Status::unavailable("the groups are not created yet"))?;
        let group = usize::try_from(group_id).ok().and_then(|index| groups.get(index));
        group.ok_or_else(|| Status::not_found(format!("no group {group_id}")))
    }
}

#[tonic::async_trait]
impl GroupRaft for RaftService {
    async fn append(
        &self,
        request: Request<pb::GroupAppendRequest>,
    ) -> Result<Response<raftpb::AppendEntriesResponse>, Status> {
        let pb::GroupAppendRequest { group_id, request } = request.into_inner();
        let request = request.ok_or_else(|| Status::invalid_argument("no AppendEntries request"))?;

        let result = self.group(group_id)?.append_entries(request.into()).await.map_err(internal)?;
        Ok(Response::new(result.into()))
    }

    async fn vote(&self, request: Request<pb::GroupVoteRequest>) -> Result<Response<raftpb::VoteResponse>, Status> {
        let pb::GroupVoteRequest { group_id, request } = request.into_inner();
        let request = request.ok_or_else(|| Status::invalid_argument("no Vote request"))?;

        let response = self.group(group_id)?.vote(request.into()).await.map_err(internal)?;
        Ok(Response::new(response.into()))
    }

    async fn pre_vote(&self, request: Request<pb::GroupVoteRequest>) -> Result<Response<raftpb::VoteResponse>, Status> {
        let pb::GroupVoteRequest { group_id, request } = request.into_inner();
        let request = request.ok_or_else(|| Status::invalid_argument("no PreVote request"))?;

        let response = self.group(group_id)?.pre_vote(request.into()).await.map_err(internal)?;
        Ok(Response::new(response.into()))
    }

    async fn snapshot(
        &self,
        request: Request<pb::GroupSnapshotRequest>,
    ) -> Result<Response<raftpb::SnapshotResponse>, Status> {
        let pb::GroupSnapshotRequest { group_id, meta, data } = request.into_inner();
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
