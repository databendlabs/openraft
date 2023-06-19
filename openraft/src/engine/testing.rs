use std::io::Cursor;

use crate::RaftTypeConfig;
use crate::TokioRuntime;

/// Trivial Raft type config for Engine related unit test.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct UTConfig {}
impl RaftTypeConfig for UTConfig {
    type D = ();
    type R = ();
    type NodeId = u64;
    type Node = ();
    type Entry = crate::Entry<UTConfig>;
    type SnapshotData = Cursor<Vec<u8>>;
    type AsyncRuntime = TokioRuntime;
}
