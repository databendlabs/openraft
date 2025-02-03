use std::sync::Arc;

use tonic::Request;
use tonic::Response;
use tonic::Status;
use tracing::debug;

use crate::protobuf::app_service_server::AppService;
use crate::protobuf::GetRequest;
use crate::protobuf::Response as PbResponse;
use crate::protobuf::SetRequest;
use crate::store::StateMachineStore;
use crate::typ::*;

/// External API service implementation providing key-value store operations.
/// This service handles client requests for getting and setting values in the distributed store.
///
/// # Responsibilities
/// - Handle key-value get operations
/// - Handle key-value set operations
/// - Ensure consistency through Raft consensus
///
/// # Protocol Safety
/// This service implements the client-facing API and should validate all inputs
/// before processing them through the Raft consensus protocol.
pub struct AppServiceImpl {
    /// The Raft node instance for consensus operations
    raft_node: Raft,
    /// The state machine store for direct reads
    state_machine_store: Arc<StateMachineStore>,
}

impl AppServiceImpl {
    /// Creates a new instance of the API service
    ///
    /// # Arguments
    /// * `raft_node` - The Raft node instance this service will use
    /// * `state_machine_store` - The state machine store for reading data
    pub fn new(raft_node: Raft, state_machine_store: Arc<StateMachineStore>) -> Self {
        AppServiceImpl {
            raft_node,
            state_machine_store,
        }
    }
}

#[tonic::async_trait]
impl AppService for AppServiceImpl {
    /// Sets a value for a given key in the distributed store
    ///
    /// # Arguments
    /// * `request` - Contains the key and value to set
    ///
    /// # Returns
    /// * `Ok(Response)` - Success response after the value is set
    /// * `Err(Status)` - Error status if the set operation fails
    async fn set(&self, request: Request<SetRequest>) -> Result<Response<PbResponse>, Status> {
        let req = request.into_inner();
        debug!("Processing set request for key: {}", req.key.clone());

        let res = self
            .raft_node
            .client_write(req.clone())
            .await
            .map_err(|e| Status::internal(format!("Failed to write to store: {}", e)))?;

        debug!("Successfully set value for key: {}", req.key);
        Ok(Response::new(res.data))
    }

    /// Gets a value for a given key from the distributed store
    ///
    /// # Arguments
    /// * `request` - Contains the key to retrieve
    ///
    /// # Returns
    /// * `Ok(Response)` - Success response containing the value
    /// * `Err(Status)` - Error status if the get operation fails
    async fn get(&self, request: Request<GetRequest>) -> Result<Response<PbResponse>, Status> {
        let req = request.into_inner();
        debug!("Processing get request for key: {}", req.key);

        let sm = self
            .state_machine_store
            .state_machine
            .lock()
            .map_err(|e| Status::internal(format!("error getting lock on sm: {}", e)))?;
        let value = sm
            .data
            .get(&req.key)
            .ok_or_else(|| Status::internal(format!("Key not found: {}", req.key)))?
            .to_string();

        debug!("Successfully retrieved value for key: {}", req.key);
        Ok(Response::new(PbResponse { value: Some(value) }))
    }
}
