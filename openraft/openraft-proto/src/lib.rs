pub mod internal_service;
pub mod management_service;

pub mod protobuf {
    tonic::include_proto!("openraftpb");
}

use openraft::error::ClientWriteError;
use openraft::error::InstallSnapshotError;
use openraft::error::RaftError;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::ClientWriteResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::BasicNode as OpenraftBasicNode;
use openraft::RaftMetrics;
use protobuf::BasicNode;
use serde::Deserialize;
use serde::Serialize;

use crate::protobuf::RaftReply;

openraft::declare_raft_types!(pub TypeConfig);

impl From<BasicNode> for OpenraftBasicNode {
    fn from(node: BasicNode) -> Self {
        OpenraftBasicNode { addr: node.addr }
    }
}

// Helper for Result to RaftReply conversion
fn convert_to_raft_reply<T: Serialize>(result: Result<T, impl ToString>) -> RaftReply {
    match result {
        Ok(response) => {
            let data = serde_json::to_string(&response).expect("Failed to serialize response");
            RaftReply {
                data,
                error: Default::default(),
            }
        }
        Err(e) => RaftReply {
            data: Default::default(),
            error: e.to_string(),
        },
    }
}

// Macro for repetitive Result to RaftReply conversions
macro_rules! impl_result_to_raft_reply {
    ($result_type:ty, $error_type:ty) => {
        impl From<Result<$result_type, $error_type>> for RaftReply {
            fn from(result: Result<$result_type, $error_type>) -> Self {
                convert_to_raft_reply(result)
            }
        }
    };
}

impl_result_to_raft_reply!(ClientWriteResponse<TypeConfig>, RaftError<TypeConfig, ClientWriteError<TypeConfig>>);
impl_result_to_raft_reply!(VoteResponse<TypeConfig>, RaftError<TypeConfig>);
impl_result_to_raft_reply!(AppendEntriesResponse<TypeConfig>, RaftError<TypeConfig>);
impl_result_to_raft_reply!(InstallSnapshotResponse<TypeConfig>, RaftError<TypeConfig, InstallSnapshotError>);

impl From<tokio::sync::watch::Receiver<RaftMetrics<TypeConfig>>> for RaftReply {
    fn from(receiver: tokio::sync::watch::Receiver<RaftMetrics<TypeConfig>>) -> Self {
        let value = receiver.borrow();
        RaftReply {
            data: serde_json::to_string(&*value).expect("Failed to serialize response"),
            error: Default::default(),
        }
    }
}

// Wrapper for deserialization
#[derive(Serialize, Deserialize)]
struct StringWrapper(String);

fn safe_deserialize_from_wrapper<T: for<'de> Deserialize<'de>>(wrapper: StringWrapper) -> Result<T, String> {
    serde_json::from_str(&wrapper.0).map_err(|e| format!("Deserialization error: {}", e))
}

// Macro for repetitive StringWrapper conversions
macro_rules! impl_from_string_wrapper {
    ($target_type:ty) => {
        impl From<StringWrapper> for $target_type {
            fn from(wrapper: StringWrapper) -> Self {
                safe_deserialize_from_wrapper(wrapper).expect("Failed to deserialize from StringWrapper")
            }
        }
    };
}

impl_from_string_wrapper!(VoteRequest<TypeConfig>);
impl_from_string_wrapper!(AppendEntriesRequest<TypeConfig>);
impl_from_string_wrapper!(InstallSnapshotRequest<TypeConfig>);
