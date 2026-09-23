use crate::RaftTypeConfig;
use crate::base::BoxOnce;
use crate::errors::Fatal;
use crate::errors::Infallible;
use crate::raft::raft_inner::RaftInner;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::WatchReceiverOf;

/// The running state of RaftCore
pub(in crate::raft) enum CoreState<C>
where C: RaftTypeConfig
{
    /// The RaftCore and its background tasks have not been spawned.
    Unstarted(Option<BoxOnce<'static, RaftInner<C>, JoinHandleOf<C, Result<Infallible, Fatal<C>>>>>),

    /// The RaftCore task is still running.
    Running(JoinHandleOf<C, Result<Infallible, Fatal<C>>>),

    /// The RaftCore task is waiting for a signal to finish joining.
    Joining(WatchReceiverOf<C, bool>),

    /// The RaftCore task has finished. The return value of the task is stored.
    Done(Result<Infallible, Fatal<C>>),
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
