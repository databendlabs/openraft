//! Type configuration for turmoil tests.

use std::io::Cursor;

use serde::Deserialize;
use serde::Serialize;

pub type NodeId = u64;

/// Node information including network address.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Node {
    pub addr: String,
}

impl std::fmt::Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.addr)
    }
}

/// Client request - a simple key-value write operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub client_id: String,
    pub serial: u64,
    pub key: String,
    pub value: String,
}

impl std::fmt::Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Request{{client:{}, serial:{}, key:{}, value:{}}}",
            self.client_id, self.serial, self.key, self.value
        )
    }
}

/// Client response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Response {
    pub value: Option<String>,
}

openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
        Node = Node,
        AsyncRuntime = openraft_rt::deterministic_rng::DeterministicRng<openraft_rt_tokio::TokioRuntime>,
);

pub type Raft = openraft::Raft<TypeConfig, std::sync::Arc<crate::store::StateMachine>>;
pub type Vote = openraft::type_config::alias::VoteOf<TypeConfig>;
pub type LogId = openraft::type_config::alias::LogIdOf<TypeConfig>;
pub type Entry = openraft::type_config::alias::EntryOf<TypeConfig>;
pub type SnapshotMeta = openraft::type_config::alias::SnapshotMetaOf<TypeConfig>;
pub type Snapshot = openraft::type_config::alias::SnapshotOf<TypeConfig>;
pub type StoredMembership = openraft::type_config::alias::StoredMembershipOf<TypeConfig>;

pub type AppendEntriesRequest = openraft::raft::AppendEntriesRequest<TypeConfig>;
pub type AppendEntriesResponse = openraft::raft::AppendEntriesResponse<TypeConfig>;
pub type VoteRequest = openraft::raft::VoteRequest<TypeConfig>;
pub type VoteResponse = openraft::raft::VoteResponse<TypeConfig>;
pub type SnapshotResponse = openraft::raft::SnapshotResponse<TypeConfig>;

pub type SnapshotData = Cursor<Vec<u8>>;
pub type RaftMetrics = openraft::RaftMetrics<TypeConfig>;
