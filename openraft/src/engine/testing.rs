use std::io::Cursor;

use crate::Node;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::impls::TokioRuntime;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::NodeIdOf;
use crate::vote::RaftLeaderIdExt;

/// Trivial Raft type config for Engine-related unit tests,
/// with an optional custom node type `N` for the Node type.
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
    type D = u64;
    type R = ();
    type NodeId = u64;
    type Node = N;
    type Term = u64;
    type LeaderId = crate::impls::leader_id_adv::LeaderId<Self>;
    type Vote = crate::impls::Vote<Self>;
    type Entry = crate::impls::Entry<Self>;
    type SnapshotData = Cursor<Vec<u8>>;
    type AsyncRuntime = TokioRuntime;
    type Responder<T>
        = crate::impls::OneshotResponder<Self, T>
    where T: OptionalSend + 'static;
}

/// Builds a log id, for testing purposes.
pub(crate) fn log_id(term: u64, node_id: NodeIdOf<UTConfig>, index: u64) -> LogIdOf<UTConfig> {
    LogIdOf::<UTConfig>::new(LeaderIdOf::<UTConfig>::new_committed(term, node_id), index)
}
