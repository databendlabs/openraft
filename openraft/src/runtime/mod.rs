use openraft_macros::add_async_trait;

use crate::RaftTypeConfig;
use crate::StorageError;
use crate::engine::Command;
use crate::storage::RaftStateMachine;

/// Defines behaviors of a runtime to support the protocol engine.
///
/// An Engine defines the consensus algorithm, i.e., what to do(`command`) when some `event`
/// happens:
///
/// It receives events such as `write-log-entry` from a client,
/// or `elect` from a timer, and outputs `command`, such as
/// `append-entry-to-storage`, or `commit-entry-at-index-5` to a runtime to execute.
///
/// A `RaftRuntime` talks to `RaftLogStorage` and `RaftNetworkV2` to get things done.
///
/// See the [Engine and Runtime Architecture guide] for the write flow and command/event loop.
///
/// [Engine and Runtime Architecture guide]: crate::docs::components::engine_runtime
#[add_async_trait]
pub(crate) trait RaftRuntime<C, SM = ()>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Run a command produced by the engine.
    ///
    /// If a command cannot be run, i.e., waiting for some event, it will be returned
    async fn run_command(&mut self, cmd: Command<C, SM>) -> Result<Option<Command<C, SM>>, StorageError<C>>;
}
