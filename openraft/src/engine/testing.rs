use std::io::Cursor;

use crate::Node;
use crate::RaftTypeConfig;
use crate::TokioRuntime;

/// Trivial Raft type config for Engine related unit tests,
/// with an optional custom node type `N` for Node type.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct UTConfig<N = ()> {
    _p: std::marker::PhantomData<N>,
}
impl<N> RaftTypeConfig for UTConfig<N>
where N: Node + Copy + Ord
{
    type D = ();
    type R = ();
    type NodeId = u64;
    type Node = N;
    type Entry = crate::Entry<Self>;
    type SnapshotData = Cursor<Vec<u8>>;
    type AsyncRuntime = TokioRuntime;
}
