use std::sync::Arc;

use crate::profile_request::ProfileRequest;

/// A KV value and the applied log index that published it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[openraft::macros::since(version = "0.10.1")]
pub struct ProfileValue {
    pub version: u64,
    pub request: Arc<ProfileRequest>,
}
