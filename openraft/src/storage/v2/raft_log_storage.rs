use std::io;

use openraft_macros::add_async_trait;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftLogReader;
use crate::RaftTypeConfig;
use crate::storage::IOFlushed;
use crate::storage::LogState;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// API for log store.
///
/// `vote` API is also included because in raft, vote is part of the log: `vote` is about **when**,
/// while `log` is about **what**. A distributed consensus is about **at what a time, happened what
/// a event**.
///
/// ## Related Types
///
/// - [`RaftStateMachine`](crate::storage::RaftStateMachine) - For state machine operations
/// - [`RaftNetworkFactory`](crate::network::RaftNetworkFactory) - For network communication
/// - [`Config`](crate::config::Config) - For configuration options
///
/// ### To ensure correctness:
///
/// - Logs must be consecutive, i.e., there must **NOT** leave a **hole** in logs.
/// - All write-IO must be serialized, i.e., the internal implementation must **NOT** apply a latter
///   write request before a former write request is completed. This rule applies to both `vote` and
///   `log` IO. E.g., Saving a vote and appending a log entry must be serialized too.
#[add_async_trait]
pub trait RaftLogStorage<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Log reader type.
    ///
    /// Log reader is used by multiple replication tasks, which read logs and send them to remote
    /// nodes.
    type LogReader: RaftLogReader<C>;

    /// Returns the last deleted log id and the last log id.
    ///
    /// The impl should **not** consider the applied log id in state machine.
    /// The returned `last_log_id` could be the log id of the last present log entry, or the
    /// `last_purged_log_id` if there is no entry at all.
    // NOTE: This can be made into sync, provided all state machines will use atomic read or the
    // like.
    async fn get_log_state(&mut self) -> Result<LogState<C>, io::Error>;

    /// Get the log reader.
    ///
    /// The method is intentionally async to give the implementation a chance to use asynchronous
    /// primitives to serialize access to the common internal object, if needed.
    async fn get_log_reader(&mut self) -> Self::LogReader;

    /// Save vote to storage.
    ///
    /// ### To ensure correctness:
    ///
    /// The vote must be persisted on disk before returning.
    async fn save_vote(&mut self, vote: &VoteOf<C>) -> Result<(), io::Error>;

    /// Saves the last committed log id to storage.
    ///
    /// # Optional feature
    ///
    /// If the state machine flushes state to disk before
    /// returning from `apply()`, then the application does not need to implement this method.
    /// Otherwise, this method is also optional(but not recommended), but your application has to
    /// deal with state reversion of state machine carefully upon restart. E.g., do not serve
    /// read operation a new `commit` message is received.
    ///
    /// See: [`docs::data::log_pointers`].
    ///
    /// [`docs::data::log_pointers`]: `crate::docs::data::log_pointers#optionally-persisted-committed`
    async fn save_committed(&mut self, _committed: Option<LogIdOf<C>>) -> Result<(), io::Error> {
        // By default `committed` log id is not saved
        Ok(())
    }

    /// Return the last saved committed log id by [`Self::save_committed`].
    async fn read_committed(&mut self) -> Result<Option<LogIdOf<C>>, io::Error> {
        // By default `committed` log id is not saved and this method just returns None.
        Ok(None)
    }

    /// Append log entries and call the `callback` once logs are persisted on disk.
    ///
    /// It should return immediately after saving the input log entries in memory and calls the
    /// `callback` when the entries are persisted on disk, i.e., avoid blocking.
    ///
    /// This method is still async because preparing the IO is usually async.
    ///
    /// ### To ensure correctness:
    ///
    /// - When this method returns, the entries must be readable, i.e., a `LogReader` can read these
    ///   entries.
    ///
    /// - When the `callback` is called, the entries must be persisted on disk.
    ///
    ///   NOTE that: the `callback` can be called either before or after this method returns.
    ///
    /// - There must not be a **hole** in logs. Because Raft only examines the last log id to ensure
    ///   correctness.
    async fn append<I>(&mut self, entries: I, callback: IOFlushed<C>) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = C::Entry> + OptionalSend,
        I::IntoIter: OptionalSend;

    /// Truncate logs since `log_id`, inclusive
    ///
    /// ### To ensure correctness:
    ///
    /// - It must not leave a **hole** in logs.
    async fn truncate(&mut self, log_id: LogIdOf<C>) -> Result<(), io::Error>;

    /// Purge logs up to `log_id`, inclusive
    ///
    /// ### To ensure correctness:
    ///
    /// - It must not leave a **hole** in logs.
    async fn purge(&mut self, log_id: LogIdOf<C>) -> Result<(), io::Error>;
}
