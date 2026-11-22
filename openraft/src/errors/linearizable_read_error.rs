use crate::RaftTypeConfig;
use crate::TryAsRef;
use crate::errors::ForwardToLeader;
use crate::errors::QuorumNotEnough;

/// An error that keeps a node from serving a linearizable read.
///
/// [`Raft::ensure_linearizable()`], [`Raft::get_read_log_id()`], [`Raft::get_read_linearizer()`]
/// and [`Raft::get_local_read_linearizer()`] return it.
///
/// [`Raft::ensure_linearizable()`]: crate::Raft::ensure_linearizable
/// [`Raft::get_read_log_id()`]: crate::Raft::get_read_log_id
/// [`Raft::get_read_linearizer()`]: crate::Raft::get_read_linearizer
/// [`Raft::get_local_read_linearizer()`]: crate::Raft::get_local_read_linearizer
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum LinearizableReadError<C>
where C: RaftTypeConfig
{
    /// This node cannot serve the read; send it to the node the error names.
    ///
    /// The node is not the leader, or it is a leader that is transferring its leadership.
    /// [`ForwardToLeader::reason`] tells which; during a transfer, the named node may not be the
    /// leader yet.
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<C>),

    /// The leader has not heard from a write quorum recently enough to serve the read.
    ///
    /// It is returned at once when the wait that the [`LinearizerOption`] allows is zero, and
    /// otherwise when that wait ends. The peers may still be reachable: once a quorum acknowledges
    /// a later heartbeat, a read can succeed.
    ///
    /// [`LinearizerOption`]: crate::LinearizerOption
    #[error(transparent)]
    QuorumNotEnough(#[from] QuorumNotEnough<C>),
}

impl<C> TryAsRef<ForwardToLeader<C>> for LinearizableReadError<C>
where C: RaftTypeConfig
{
    fn try_as_ref(&self) -> Option<&ForwardToLeader<C>> {
        match self {
            Self::ForwardToLeader(f) => Some(f),
            _ => None,
        }
    }
}
