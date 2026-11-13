use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::TryAsRef;
use crate::errors::ForwardToLeader;
use crate::errors::LinearizableReadError;
use crate::type_config::alias::LogIdOf;

/// A relaxed read was disabled, could not confirm leadership, or sampled an unsafe snapshot.
#[since(version = "0.10.1")]
#[derive(Debug, Clone, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum RelaxedReadError<C>
where C: RaftTypeConfig
{
    /// Enable `Config::enable_relaxed_read` only after stopping and draining follower reads.
    #[error("relaxed reads are disabled; enable_relaxed_read must be Some(true)")]
    Disabled,

    /// Leadership could not be confirmed using the requested quorum or lease options.
    #[error(transparent)]
    LinearizableRead(#[from] LinearizableReadError<C>),

    /// The sampled snapshot is below the no-op barrier or belongs to another leadership.
    ///
    /// Discard the query result and retry with fresh authorization. In particular, an authorized
    /// read may outlive step-down and sample entries applied as a follower of the next leader.
    #[error("unsafe relaxed-read snapshot: expected leadership and progress of {expected}, sampled {applied:?}")]
    InvalidSnapshot {
        /// The authorizing leadership's no-op log ID, which must have been applied.
        expected: LogIdOf<C>,
        /// The applied log ID sampled coherently with the query result.
        applied: Option<LogIdOf<C>>,
    },
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
