use std::collections::BTreeMap;

use tonic::Request;
use tonic::Response;
use tonic::Status;
use tracing::debug;

use crate::pb;
use crate::protobuf::management_service_server::ManagementService;
use crate::typ::*;

/// Management service implementation for Raft cluster administration.
/// Handles cluster initialization, membership changes, and metrics collection.
///
/// # Responsibilities
/// - Cluster initialization
/// - Adding learner nodes
/// - Changing cluster membership
/// - Collecting metrics
pub struct ManagementServiceImpl {
    raft_node: Raft,
}

impl ManagementServiceImpl {
    /// Creates a new instance of the management service
    ///
    /// # Arguments
    /// * `raft_node` - The Raft node instance this service will manage
    pub fn new(raft_node: Raft) -> Self {
        ManagementServiceImpl { raft_node }
    }
}

#[tonic::async_trait]
impl ManagementService for ManagementServiceImpl {
    /// Initializes a new Raft cluster with the specified nodes
    ///
    /// # Arguments
    /// * `request` - Contains the initial set of nodes for the cluster
    ///
    /// # Returns
    /// * Success response with initialization details
    /// * Error if initialization fails
    async fn init(&self, request: Request<pb::InitRequest>) -> Result<Response<()>, Status> {
        debug!("Initializing Raft cluster");
        let req = request.into_inner();

        // Convert nodes into required format
        let nodes_map: BTreeMap<u64, pb::Node> = req
            .nodes
            .into_iter()
            .map(|node| {
                (node.node_id, pb::Node {
                    rpc_addr: node.rpc_addr,
                    node_id: node.node_id,
                })
            })
            .collect();

        // Initialize the cluster
        let result = self
            .raft_node
            .initialize(nodes_map)
            .await
            .map_err(|e| Status::internal(format!("Failed to initialize cluster: {}", e)))?;

        debug!("Cluster initialization successful");
        Ok(Response::new(result))
    }

    /// Adds a learner node to the Raft cluster
    ///
    /// # Arguments
    /// * `request` - Contains the node information and blocking preference
    ///
    /// # Returns
    /// * Success response with learner addition details
    /// * Error if the operation fails
    async fn add_learner(
        &self,
        request: Request<pb::AddLearnerRequest>,
    ) -> Result<Response<pb::ClientWriteResponse>, Status> {
        let req = request.into_inner();

        let node = req.node.ok_or_else(|| Status::internal("Node information is required"))?;

        debug!("Adding learner node {}", node.node_id);

        let raft_node = Node {
            rpc_addr: node.rpc_addr.clone(),
            node_id: node.node_id,
        };

        let result = self
            .raft_node
            .add_learner(node.node_id, raft_node, true)
            .await
            .map_err(|e| Status::internal(format!("Failed to add learner node: {}", e)))?;

        debug!("Successfully added learner node {}", node.node_id);
        Ok(Response::new(result.into()))
    }

    /// Changes the membership of the Raft cluster
    ///
    /// # Arguments
    /// * `request` - Contains the new member set and retention policy
    ///
    /// # Returns
    /// * Success response with membership change details
    /// * Error if the operation fails
    async fn change_membership(
        &self,
        request: Request<pb::ChangeMembershipRequest>,
    ) -> Result<Response<pb::ClientWriteResponse>, Status> {
        let req = request.into_inner();

        debug!(
            "Changing membership. Members: {:?}, Retain: {}",
            req.members, req.retain
        );

        let result = self
            .raft_node
            .change_membership(req.members, req.retain)
            .await
            .map_err(|e| Status::internal(format!("Failed to change membership: {}", e)))?;

        debug!("Successfully changed cluster membership");
        Ok(Response::new(result.into()))
    }

    /// Retrieves metrics about the Raft node
    ///
    /// # Returns
    /// * Success response with metrics data
    /// * Error if metrics collection fails
    async fn metrics(&self, _request: Request<()>) -> Result<Response<pb::MetricsString>, Status> {
        debug!("Collecting metrics");
        let metrics = self.raft_node.metrics().borrow().clone();
        let resp = pb::MetricsString {
            data: metrics.to_string(),
        };

        Ok(Response::new(resp))
    }
}
