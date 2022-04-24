/// Req for test
#[derive(Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Req {}

/// Resp for test
#[derive(Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Resp {}

// Config for test
crate::declare_raft_types!(
   pub(crate) Config: D = Req, R = Resp, NodeId = u64
);
