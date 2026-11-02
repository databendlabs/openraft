use futures_util::FutureExt;
use futures_util::future::Shared;

use crate::RaftTypeConfig;
use crate::async_runtime::AsyncRuntime;
use crate::base::BoxFuture;
use crate::base::BoxOnce;
use crate::errors::Fatal;
use crate::errors::Infallible;
use crate::raft::raft_inner::RaftInner;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::type_config::alias::JoinHandleOf;

/// The running state of RaftCore
pub(in crate::raft) enum CoreState<C>
where C: RaftTypeConfig
{
    /// The RaftCore and its background tasks have not been spawned.
    Unstarted(Option<BoxOnce<'static, RaftInner<C>, JoinHandleOf<C, Result<Infallible, Fatal<C>>>>>),

    /// The RaftCore task was spawned; its shared join retains the result after completion.
    Started(Shared<BoxFuture<'static, (Result<Infallible, Fatal<C>>, bool)>>),

    /// The node was shut down before its tasks were spawned.
    StoppedBeforeRun,
}

impl<C> CoreState<C>
where C: RaftTypeConfig
{
    pub(in crate::raft) fn started(handle: JoinHandleOf<C, Result<Infallible, Fatal<C>>>) -> Self {
        let future: BoxFuture<'static, _> = Box::pin(async move {
            let result = handle.await;
            tracing::info!("RaftCore exited: {:?}", result);
            let join_failed = result.is_err();
            let core_result = match result {
                Ok(returned) => returned,
                Err(err) if AsyncRuntimeOf::<C>::is_panic(&err) => Err(Fatal::Panicked),
                Err(_) => Err(Fatal::Stopped),
            };
            (core_result, join_failed)
        });
        let join = future.shared();
        Self::Started(join)
    }
}
