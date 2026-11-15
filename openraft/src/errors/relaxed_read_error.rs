use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::TryAsRef;
use crate::errors::ForwardToLeader;
use crate::errors::LinearizableReadError;

/// A relaxed read was disabled or could not confirm leadership.
#[since(version = "0.10.1")]
#[derive(Debug, Clone, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum RelaxedReadError<C>
where C: RaftTypeConfig
{
    /// `Config::enable_relaxed_read` is not `Some(true)` on this node.
    ///
    /// Enable it on every node before serving relaxed reads anywhere in the cluster.
    #[error("relaxed reads are disabled; enable_relaxed_read must be Some(true)")]
    Disabled,

    /// Leadership could not be confirmed using the requested quorum or lease options.
    #[error(transparent)]
    LinearizableRead(#[from] LinearizableReadError<C>),
}

impl<C> TryAsRef<ForwardToLeader<C>> for RelaxedReadError<C>
where C: RaftTypeConfig
{
    fn try_as_ref(&self) -> Option<&ForwardToLeader<C>> {
        match self {
            Self::LinearizableRead(e) => e.try_as_ref(),
            _ => None,
        }
    }
}
