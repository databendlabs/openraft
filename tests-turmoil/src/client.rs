//! Client types for the RPC protocol.

use serde::Deserialize;
use serde::Serialize;

use crate::typ::*;

/// Client request to be sent to a Raft node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientWriteRequest {
    pub request: Request,
}

/// Client response from a Raft node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientWriteResponse {
    Success(Response),
    NotLeader { leader_id: Option<NodeId> },
    Error(String),
}
