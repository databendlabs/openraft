use crate::RaftTypeConfig;

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
   pub(crate) Config: D = Req, R = Resp, NodeId = u64, Node=(), Entry = crate::Entry<Config>
);

/// Trivial Raft type config for Engine related unit test.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct UTCfg {}
impl RaftTypeConfig for UTCfg {
    type D = ();
    type R = ();
    type NodeId = u64;
    type Node = ();
    type Entry = crate::Entry<UTCfg>;
}
