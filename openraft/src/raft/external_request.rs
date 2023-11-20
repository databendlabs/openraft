//! Defines API for application to send request to access Raft core.

use crate::type_config::alias::InstantOf;
use crate::type_config::alias::NodeIdOf;
use crate::type_config::alias::NodeOf;
use crate::RaftState;

/// Boxed trait object for external request function run in `RaftCore` task.
pub(crate) type BoxCoreFn<C> = Box<dyn FnOnce(&RaftState<NodeIdOf<C>, NodeOf<C>, InstantOf<C>>) + Send + 'static>;
