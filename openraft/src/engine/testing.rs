/// Req for test
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct Req {}

/// Resp for test
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct Resp {}

// Config for test
crate::declare_raft_types!(
   pub(crate) Config: D = Req, R = Resp, NodeId = u64
);
