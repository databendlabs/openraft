use crate::error::Fatal;
use crate::AsyncRuntime;
use crate::NodeId;

/// The running state of RaftCore
pub(in crate::raft) enum CoreState<NID, A>
where
    NID: NodeId,
    A: AsyncRuntime,
{
    /// The RaftCore task is still running.
    Running(A::JoinHandle<Result<(), Fatal<NID>>>),

    /// The RaftCore task has finished. The return value of the task is stored.
    Done(Result<(), Fatal<NID>>),
}
