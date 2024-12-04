use std::collections::BTreeMap;
use std::sync::Arc;

use openraft::BasicNode as OpenraftBasicNode;
use openraft::ChangeMembers;
use tonic::Request;
use tonic::Response;
use tracing::debug;

use crate::protobuf::management_service_server::ManagementService;
use crate::protobuf::AddLearnerRequest;
use crate::protobuf::ChangeMembershipRequest;
use crate::protobuf::InitRequest;
use crate::protobuf::InitResponse;
use crate::protobuf::MetricsRequest;
use crate::protobuf::RaftReply;
use crate::TypeConfig;

pub struct RaftManagementService {
    raft: Arc<openraft::Raft<TypeConfig>>,
}

impl RaftManagementService {
    pub async fn new(raft: Arc<openraft::Raft<TypeConfig>>) -> Result<Self, openraft::error::RaftError<TypeConfig>> {
        Ok(Self { raft })
    }
}

#[tonic::async_trait]
impl ManagementService for RaftManagementService {
    async fn init(&self, request: Request<InitRequest>) -> Result<Response<InitResponse>, tonic::Status> {
        let init_request = request.into_inner();
        debug!("Init request received: {:?}", init_request);

        let nodes_map: BTreeMap<u64, OpenraftBasicNode> = init_request
            .nodes
            .into_iter()
            .map(|node| (node.id.into(), OpenraftBasicNode::from(node).into()))
            .collect();
        match self.raft.initialize(nodes_map).await {
            Ok(_) => {
                let response = InitResponse {
                    status: Some(crate::protobuf::Status {
                        code: 0,
                        message: "success".to_string(),
                    }),
                };
                Ok(Response::new(response))
            }
            Err(e) => Err(tonic::Status::internal(format!("Failed to initialize: {}", e))),
        }
    }

    async fn add_learner(&self, request: Request<AddLearnerRequest>) -> Result<Response<RaftReply>, tonic::Status> {
        let add_learner_request = request.into_inner();
        debug!("AddLearner request received: {:?}", add_learner_request);
        let node_id = add_learner_request.node.clone().unwrap_or_default().id;
        let node = OpenraftBasicNode::from(add_learner_request.node.unwrap_or_default());

        let result = self.raft.add_learner(node_id, node, true).await;
        let reply: RaftReply = result.into();

        Ok(Response::new(reply))
    }

    async fn change_membership(
        &self,
        request: Request<ChangeMembershipRequest>,
    ) -> Result<Response<RaftReply>, tonic::Status> {
        let change_membership_request = request.into_inner();
        debug!("ChangeMembership request received: {:?}", change_membership_request);

        let nodes_map = change_membership_request
            .nodes
            .into_iter()
            .map(|node| (node.id.into(), OpenraftBasicNode::from(node).into()))
            .collect();

        let change_members = ChangeMembers::ReplaceAllNodes(nodes_map);

        // Call the change_membership method on the Raft instance
        let result = self.raft.change_membership(change_members, true).await;

        let reply: RaftReply = result.into();

        Ok(Response::new(reply))
    }

    async fn metrics(&self, request: Request<MetricsRequest>) -> Result<Response<RaftReply>, tonic::Status> {
        let metrics_request = request.into_inner();
        debug!("Metrics request received: {:?}", metrics_request);

        let result = self.raft.metrics();
        let reply: RaftReply = result.into();

        Ok(Response::new(reply))
    }
}
