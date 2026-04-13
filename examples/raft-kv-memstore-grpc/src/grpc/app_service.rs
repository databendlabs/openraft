use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;

use openraft::async_runtime::WatchReceiver;
use openraft::errors::decompose::DecomposeResult;
use tonic::Request;
use tonic::Response;
use tonic::Status;
use tonic::metadata::MetadataValue;
use tracing::debug;

use crate::pb;
use crate::protobuf::GetRequest;
use crate::protobuf::Response as PbResponse;
use crate::protobuf::SetRequest;
use crate::protobuf::app_service_server::AppService;
use crate::store::StateMachineStore;
use crate::typ::*;

/// gRPC metadata header used to communicate the leader's endpoint to clients.
///
/// When a non-leader node receives a write request, it returns `Status::unavailable`
/// with this header set to the leader's address, so clients can retry against the leader.
pub const LEADER_ENDPOINT_HEADER: &str = "x-openraft-leader-endpoint";

/// Build a `Status::unavailable` with the leader's endpoint in gRPC metadata,
/// so the client knows where to retry.
fn status_forward_to_leader(forward: &ForwardToLeader) -> Status {
    let mut status = Status::unavailable(format!("{}", forward));

    if let Some(ref node) = forward.leader_node
        && let Ok(v) = MetadataValue::from_str(&node.rpc_addr)
    {
        status.metadata_mut().insert(LEADER_ENDPOINT_HEADER, v);
    }

    status
}

/// External API service providing key-value store operations over gRPC.
pub struct AppServiceImpl {
    raft_node: Raft,
    state_machine_store: Arc<StateMachineStore>,
}

impl AppServiceImpl {
    pub fn new(raft_node: Raft, state_machine_store: Arc<StateMachineStore>) -> Self {
        AppServiceImpl {
            raft_node,
            state_machine_store,
        }
    }
}

#[tonic::async_trait]
impl AppService for AppServiceImpl {
    async fn set(&self, request: Request<SetRequest>) -> Result<Response<PbResponse>, Status> {
        let req = request.into_inner();
        debug!("Processing set request for key: {}", req.key);

        let res = self.raft_node.client_write(req.clone()).await;

        match res.decompose() {
            Ok(Ok(resp)) => Ok(Response::new(resp.data)),
            Ok(Err(ClientWriteError::ForwardToLeader(forward))) => Err(status_forward_to_leader(&forward)),
            Ok(Err(e)) => Err(Status::internal(e.to_string())),
            Err(fatal) => Err(Status::internal(fatal.to_string())),
        }
    }

    async fn get(&self, request: Request<GetRequest>) -> Result<Response<PbResponse>, Status> {
        let req = request.into_inner();
        debug!("Processing get request for key: {}", req.key);

        let sm = self.state_machine_store.state_machine.lock().await;
        let value = sm.data.get(&req.key).map(|v| v.to_string());

        Ok(Response::new(PbResponse { value }))
    }

    async fn init(&self, request: Request<pb::InitRequest>) -> Result<Response<()>, Status> {
        let req = request.into_inner();

        let nodes_map: BTreeMap<u64, pb::Node> = req.nodes.into_iter().map(|node| (node.node_id, node)).collect();

        self.raft_node.initialize(nodes_map).await.map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(()))
    }

    async fn add_learner(
        &self,
        request: Request<pb::AddLearnerRequest>,
    ) -> Result<Response<pb::ClientWriteResponse>, Status> {
        let req = request.into_inner();
        let node = req.node.ok_or_else(|| Status::invalid_argument("Node information is required"))?;

        let raft_node = Node {
            rpc_addr: node.rpc_addr.clone(),
            node_id: node.node_id,
        };

        let res = self.raft_node.add_learner(node.node_id, raft_node, true).await;

        match res.decompose() {
            Ok(Ok(resp)) => Ok(Response::new(resp.into())),
            Ok(Err(ClientWriteError::ForwardToLeader(forward))) => Err(status_forward_to_leader(&forward)),
            Ok(Err(e)) => Err(Status::internal(e.to_string())),
            Err(fatal) => Err(Status::internal(fatal.to_string())),
        }
    }

    async fn change_membership(
        &self,
        request: Request<pb::ChangeMembershipRequest>,
    ) -> Result<Response<pb::ClientWriteResponse>, Status> {
        let req = request.into_inner();

        let res = self.raft_node.change_membership(req.members, req.retain).await;

        match res.decompose() {
            Ok(Ok(resp)) => Ok(Response::new(resp.into())),
            Ok(Err(ClientWriteError::ForwardToLeader(forward))) => Err(status_forward_to_leader(&forward)),
            Ok(Err(e)) => Err(Status::internal(e.to_string())),
            Err(fatal) => Err(Status::internal(fatal.to_string())),
        }
    }

    async fn metrics(&self, _request: Request<()>) -> Result<Response<pb::MetricsResponse>, Status> {
        let metrics = self.raft_node.metrics().borrow_watched().clone();
        let resp = pb::MetricsResponse {
            membership: Some(metrics.membership_config.membership().clone().into()),
            other_metrics: metrics.to_string(),
        };
        Ok(Response::new(resp))
    }
}
