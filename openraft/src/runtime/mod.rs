use crate::engine::Command;
use crate::raft_types::RaftLogId;
use crate::Entry;
use crate::RaftTypeConfig;
use crate::StorageError;

/// Defines behaviors of a runtime to support the protocol engine.
///
/// An Engine defines the consensus algorithm, i.e., what to do(`command`) when some `event` happens:
///
/// It receives events such as `write-log-entry` from a client,
/// or `elect` from a timer, and outputs `command`, such as
/// `append-entry-to-storage`, or `commit-entry-at-index-5` to a runtime to execute.
///
/// A `RaftRuntime` talks to `RaftStorage` and `RaftNetwork` to get things done.
///
/// The workflow of writing something through raft protocol with engine and runtime would be like
/// this:
///
/// ```text
/// Client                    Engine                         Runtime      Storage      Netwoork
///   |                         |      write x=1                |            |            |
///   |-------------------------------------------------------->|            |            |
///   |        event:write      |                               |            |            |
///   |        .------------------------------------------------|            |            |
///   |        '--------------->|                               |            |            |
///   |                         |      cmd:append-log-1         |            |            |
///   |                         |--+--------------------------->|   append   |            |
///   |                         |  |                            |----------->|            |
///   |                         |  |                            |<-----------|            |
///   |                         |  |   cmd:replicate-log-1      |   ok       |            |
///   |                         |  '--------------------------->|            |            |
///   |                         |                               |            |   send     |
///   |                         |                               |------------------------>|
///   |                         |                               |            |   send     |
///   |                         |                               |------------------------>|
///   |                         |                               |            |            |
///   |                         |                               |<------------------------|
///   |         event:ok        |                               |   ok       |            |
///   |        .------------------------------------------------|            |            |
///   |        '--------------->|                               |            |            |
///   |                         |                               |<------------------------|
///   |         event:ok        |                               |   ok       |            |
///   |        .------------------------------------------------|            |            |
///   |        '--------------->|                               |            |            |
///   |                         |     cmd:commit-log-1          |            |            |
///   |                         |------------------------------>|            |            |
///   |                         |     cmd:apply-log-1           |            |            |
///   |                         |------------------------------>|            |            |
///   |                         |                               |   apply    |            |
///   |                         |                               |----------->|            |
///   |                         |                               |<-----------|            |
///   |                         |                               |   ok       |            |
///   |<--------------------------------------------------------|            |            |
///   |        response         |                               |            |            |
/// ```
///
/// TODO: add this diagram to guides/
#[async_trait::async_trait]
pub(crate) trait RaftRuntime<C: RaftTypeConfig> {
    /// Run a command produced by the engine.
    ///
    /// A command consumes zero or more input entries.
    /// `curr` points to next non consumed entry in the `input_entries`.
    /// It's the command's duty to decide move `curr` forward.
    ///
    /// TODO(xp): remove this method. The API should run all commands in one shot.
    ///           E.g. `run_engine_commands(input_entries, commands)`.
    ///           But it relates to the different behaviors of server states.
    ///           LeaderState is a wrapper of RaftCore thus it can not just reuse `RaftCore::run_engine_commands()`.
    ///           Thus LeaderState may have to re-implememnt every command.
    ///           This can be done after moving all raft-algorithm logic into Engine.
    ///           Then a Runtime do not need to differentiate states such as LeaderState or FollowerState and all
    ///           command execution can be implemented in one method.
    async fn run_command<'e, Ent>(
        &mut self,
        input_entries: &'e [Ent],
        curr: &mut usize,
        cmd: &Command<C::NodeId, C::Node>,
    ) -> Result<(), StorageError<C::NodeId>>
    where
        Ent: RaftLogId<C::NodeId> + Sync + Send + 'e,
        &'e Ent: Into<Entry<C>>;
}
