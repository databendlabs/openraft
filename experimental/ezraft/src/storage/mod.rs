//! Persistence: the [`EzStorage`] contract, the values it exchanges, and one implementation
//!
//! [`FileStorage`] is here to get a first cluster running, not to be the storage a deployment
//! keeps. Read its caveats before reaching for it, and implement [`EzStorage`] yourself once
//! they matter.

pub mod adapter;
mod file_storage;
mod loaded;
mod persist;

use std::io;

use async_trait::async_trait;
pub use file_storage::FileStorage;
pub use loaded::Loaded;
pub use persist::Persist;

use crate::app::EzApp;
use crate::entry::EzEntry;

/// Storage persistence trait
///
/// Implement this to handle how Raft state is persisted to disk.
/// The framework handles all Raft logic - you only handle serialization and I/O.
///
/// # What the framework assumes
///
/// Raft's safety guarantees rest on these, so an implementation that breaks one can lose
/// acknowledged writes or elect two leaders for the same term:
///
/// - **Durability.** [`persist`] must return only once the data would survive a crash of the
///   machine, not just of the process. Anything weaker means a node can forget a vote it cast or a
///   log entry it acknowledged. The bundled [`FileStorage`] writes files without `fsync`, which is
///   fine for a demo and not enough for a real deployment.
/// - **Ordering.** Operations are applied in the order [`persist`] receives them. A later one must
///   not become durable before an earlier one.
/// - **Read-your-writes.** [`read_logs`] returns what [`persist`] last wrote, including the
///   deletions requested by [`Persist::DeleteLogs`].
///
/// [`persist`]: Self::persist
/// [`read_logs`]: Self::read_logs
///
/// # Example (file-based storage)
///
/// [`FileStorage`] is this sketch filled in, and is worth reading before writing your own.
///
/// ```ignore
/// struct MyStorage { base_dir: PathBuf }
///
/// #[async_trait]
/// impl EzStorage<KvApp> for MyStorage {
///     async fn load(&mut self) -> Result<Loaded, io::Error> {
///         // 1. Load meta from base_dir/meta.json (use default if first run)
///         // 2. Optionally load snapshot from base_dir/snapshot.meta + snapshot.data
///         // Log entries are read separately via read_logs()
///         Ok(Loaded { meta, snapshot })
///     }
///
///     async fn persist(&mut self, op: Persist<KvApp>) -> Result<(), io::Error> {
///         match op {
///             Persist::Meta(meta) => { /* write meta */ }
///             Persist::LogEntry(entry) => { /* write log entry */ }
///             Persist::Snapshot(snapshot) => { /* write snapshot.meta and snapshot.snapshot */ }
///             Persist::DeleteLogs { from, to } => { /* delete entries with from <= index < to */ }
///         }
///     }
///
///     async fn read_logs(&mut self, start: u64, end: u64) -> Result<Vec<EzEntry<KvApp>>, io::Error> {
///         // Read log entries in range [start, end)
///     }
/// }
/// ```
#[async_trait]
pub trait EzStorage<T>: Send + Sync + 'static
where T: EzApp
{
    /// Load metadata and snapshot on startup
    ///
    /// Returns the persisted [`Loaded`] state: metadata (or default if first run) and optional
    /// snapshot. Log entries are read separately via [`Self::read_logs`].
    ///
    /// Called exactly once, before the node starts. The framework keeps the snapshot it gets
    /// here, so serving one to a lagging peer does not call back into this method.
    async fn load(&mut self) -> Result<Loaded, io::Error>;

    /// Persist a state update
    ///
    /// Each call represents one atomic operation that should be durably persisted.
    /// The framework calls this method when state changes. See [`Persist`] for what each
    /// operation requires.
    async fn persist(&mut self, op: Persist<T>) -> Result<(), io::Error>;

    /// Read log entries within a specific index range
    ///
    /// Returns log entries where `start <= entry.index < end`.
    /// Called during replication to read specific entries without loading all logs.
    ///
    /// # Arguments
    /// * `start` - Start index (inclusive)
    /// * `end` - End index (exclusive)
    ///
    /// # Returns
    /// Log entries in the range, sorted by index. Empty vec if range is empty or
    /// no entries exist in range.
    async fn read_logs(&mut self, start: u64, end: u64) -> Result<Vec<EzEntry<T>>, io::Error>;
}
