use crate::DummyNetwork;
use crate::DummyStorage;

/// Req for test
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct Req {}

/// Resp for test
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct Resp {}

// Config for test
crate::declare_raft_types!(
   pub(crate) Config: D = Req, R = Resp, S = DummyStorage<Self>, N = DummyNetwork<Self>, NodeId = u64
);
