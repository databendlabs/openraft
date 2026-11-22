use openraft::RaftTypeConfig;
use openraft::alias::LogIdOf;
use openraft::macros::since;
use serde::Deserialize;
use serde::Serialize;

/// What a node reports about its own state, read from its metrics without contacting another
/// node.
///
/// Each field answers a different question, and none of them authorizes a request. A leader cut
/// off from its quorum keeps reporting `is_leader` until it learns of a newer leader. A
/// linearizable read needs its own read barrier: [`App::ensure_linearizable`] on the leader, or
/// [`App::ensure_follower_read_ready`] on any node. A leader decides whether to admit a write when
/// the write arrives, and rejects it with a [`ForwardReason`].
///
/// [`App::ensure_linearizable`]: crate::App::ensure_linearizable
/// [`App::ensure_follower_read_ready`]: crate::App::ensure_follower_read_ready
/// [`ForwardReason`]: openraft::errors::ForwardReason
#[since(version = "0.10.2")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound = "")]
pub struct Readiness<C>
where C: RaftTypeConfig
{
    /// The leader that this node's committed vote names, if any.
    ///
    /// Route requests to it. It may have lost its quorum without knowing it yet.
    pub leader: Option<C::NodeId>,

    /// Whether this node is `leader`, the answer [`Raft::as_leader()`] gives.
    ///
    /// [`Raft::as_leader()`]: openraft::Raft::as_leader
    pub is_leader: bool,

    /// Whether this node has applied the no-op entry that `leader` proposed first.
    ///
    /// Once a node applies it, the node's state machine holds every entry committed before that
    /// leadership began. Until then, even the leader's own state machine may lack entries that its
    /// predecessor committed.
    pub applied_leader_noop: bool,

    /// The last log ID this node applied.
    pub last_applied: Option<LogIdOf<C>>,
}
