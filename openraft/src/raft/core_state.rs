use crate::RaftTypeConfig;
use crate::error::Fatal;
use crate::error::Infallible;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::WatchReceiverOf;

/// The running state of RaftCore
pub(in crate::raft) enum CoreState<C>
where C: RaftTypeConfig
{
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
    pub(in crate::raft) fn is_running(&self) -> bool {
        matches!(self, CoreState::Running(_))
    }
}
