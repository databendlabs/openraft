use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::AsyncRuntime;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::type_config::alias::JoinErrorOf;

/// The recorded outcome of joining a background task.
///
/// This describes the runtime join, independently of the value returned by the task.
#[since(version = "0.10.0", change = "added background task join outcomes")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskJoinOutcome {
    /// The task returned and its join succeeded.
    Joined,
    /// The runtime reported that the task panicked.
    Panicked,
    /// The runtime reported a non-panic join failure, including cancellation or abort.
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
