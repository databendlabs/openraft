//! Defines API for application to send request to access Raft core.

use crate::OptionalSend;
use crate::RaftState;
use crate::RaftTypeConfig;

pub trait BoxCoreFnInternal<C>: FnOnce(&RaftState<C>) + OptionalSend
where C: RaftTypeConfig
{
}

impl<C: RaftTypeConfig, T: FnOnce(&RaftState<C>) + OptionalSend> BoxCoreFnInternal<C> for T {}

/// Boxed trait object for external request function run in `RaftCore` task.
pub(crate) type BoxCoreFn<C> = Box<dyn BoxCoreFnInternal<C> + 'static>;
