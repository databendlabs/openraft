use futures_util::future::Shared;

use crate::RaftTypeConfig;
use crate::base::BoxFuture;
use crate::base::BoxOnce;
use crate::errors::Fatal;
use crate::errors::Infallible;
use crate::raft::TaskJoinOutcome;
use crate::raft::raft_inner::RaftInner;
use crate::type_config::alias::JoinHandleOf;

/// The running state of RaftCore
pub(in crate::raft) enum CoreState<C>
where C: RaftTypeConfig
{
    /// The RaftCore and its background tasks have not been spawned.
    Unstarted(Option<BoxOnce<'static, RaftInner<C>, JoinHandleOf<C, Result<Infallible, Fatal<C>>>>>),

    /// The RaftCore task is still running.
    Running(JoinHandleOf<C, Result<Infallible, Fatal<C>>>),

    /// Callers share the in-progress runtime join of the RaftCore task.
    Joining(Shared<BoxFuture<'static, (Result<Infallible, Fatal<C>>, TaskJoinOutcome)>>),

    /// The RaftCore task has finished. Its return value and join outcome are stored.
    Done(Result<Infallible, Fatal<C>>, TaskJoinOutcome),
}

impl<C> CoreState<C>
where C: RaftTypeConfig
{
    /// Returns `true` if the RaftCore task is still running.
    #[allow(dead_code)]
    pub(in crate::raft) fn is_running(&self) -> bool {
        matches!(self, CoreState::Running(_))
    }
}
