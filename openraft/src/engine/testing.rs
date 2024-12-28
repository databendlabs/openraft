use std::io::Cursor;

use crate::impls::TokioRuntime;
use crate::Node;
use crate::RaftTypeConfig;

/// Trivial Raft type config for Engine related unit tests,
/// with an optional custom node type `N` for Node type.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct UTConfig<N = ()> {
    _p: std::marker::PhantomData<N>,
}

impl<N> Default for UTConfig<N> {
    fn default() -> Self {
        Self {
            _p: std::marker::PhantomData,
        }
    }
}

impl<N> Clone for UTConfig<N> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<N> Copy for UTConfig<N> {}

impl<N> RaftTypeConfig for UTConfig<N>
where N: Node + Ord
{
    type D = ();
    type R = ();
    type NodeId = u64;
    type Node = N;
    type Entry = crate::Entry<Self>;
    type Term = u64;
    type LeaderId = crate::impls::leader_id_adv::LeaderId<Self>;
    type SnapshotData = Cursor<Vec<u8>>;
    type AsyncRuntime = TokioRuntime;
    type Responder = crate::impls::OneshotResponder<Self>;
}
