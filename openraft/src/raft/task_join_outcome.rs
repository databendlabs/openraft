use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::AsyncRuntime;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::type_config::alias::JoinErrorOf;

/// The recorded outcome of joining a background task.
///
/// This describes the runtime join, independently of the value returned by the task.
#[since(version = "0.10.0")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskJoinOutcome {
    /// The task returned and its join succeeded.
    Joined,
    /// The runtime join failed and [`AsyncRuntime::is_panic`] classified the failure as a panic.
    Panicked,
    /// The runtime join failed and [`AsyncRuntime::is_panic`] classified the failure as non-panic.
    ///
    /// Cancellation and abort classification depends on the runtime.
    Failed,
    /// The task was never spawned.
    NotStarted,
}

impl TaskJoinOutcome {
    pub(crate) fn from_result<C, T>(result: &Result<T, JoinErrorOf<C>>) -> Self
    where C: RaftTypeConfig {
        match result {
            Ok(_) => Self::Joined,
            Err(err) if AsyncRuntimeOf::<C>::is_panic(err) => Self::Panicked,
            Err(_) => Self::Failed,
        }
    }
}
