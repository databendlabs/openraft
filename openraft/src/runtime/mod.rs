use crate::engine::Command;
use crate::entry::EntryRef;
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
    async fn run_command<'p>(
        &mut self,
        input_entries: &[EntryRef<'p, C>],
        curr: &mut usize,
        cmd: &Command<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>>;
}
