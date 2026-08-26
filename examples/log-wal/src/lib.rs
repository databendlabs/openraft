//! Provide a write-ahead-log backed log storage implementation for examples.
//!
//! [`WalLogStore`] stores Raft log entries in [`raft_log`], a write-ahead log
//! written for the access pattern a Raft log has: append at the tail, truncate
//! a suffix, purge a prefix. Records go into chunk files in write order, and
//! the store keeps the log id, vote, committed id and purged id that openraft
//! asks for in the same stream.

#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]

use std::fmt::Debug;
use std::fs;
use std::io;
use std::ops::Bound;
use std::ops::RangeBounds;
use std::sync::Arc;

use openraft::LogIdOptionExt;
use openraft::LogState;
use openraft::OptionalSend;
use openraft::RaftLogReader;
use openraft::RaftTypeConfig;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::VoteOf;
use openraft::entry::RaftEntry;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;
use raft_log::Config;
use raft_log::RaftLog;
use raft_log::api::raft_log_writer::RaftLogWriter;
use tokio::sync::RwLock;
use tokio::sync::oneshot;

#[cfg(test)]
mod test;

mod callback;
mod codec;
mod types;

pub use callback::Callback;
pub use codec::MsgPack;
pub use codec::MsgPackVote;
pub use types::WalTypes;

/// A [`raft_log`]-backed implementation of [`RaftLogStorage`].
///
/// Every clone shares one [`RaftLog`] behind an async `RwLock`. openraft reads
/// entries through the clone that [`RaftLogStorage::get_log_reader`] returns,
/// so a read for replication takes the shared lock and runs while other reads
/// run. Only the writing methods take the lock exclusively.
#[derive(Debug, Clone)]
pub struct WalLogStore<C>
where
    C: RaftTypeConfig,
    EntryOf<C>: Clone,
{
    inner: Arc<RwLock<RaftLog<WalTypes<C>>>>,
}

impl<C> WalLogStore<C>
where
    C: RaftTypeConfig,
    EntryOf<C>: Clone,
{
    /// Open the log in `dir`, creating `dir` and the log in it when they do
    /// not exist yet.
    pub fn open(dir: impl ToString) -> Result<Self, io::Error> {
        let config = Config::new(dir);
        Self::open_with_config(config)
    }

    /// Open the log with a caller-built [`Config`], which sets the directory,
    /// the chunk size limits and the payload cache size.
    pub fn open_with_config(config: Config) -> Result<Self, io::Error> {
        // `raft_log` creates the lock file in this directory but not the
        // directory itself, so a first start on a fresh path fails without
        // this.
        fs::create_dir_all(&config.wal.dir)?;

        let raft_log = RaftLog::open(Arc::new(config))?;

        Ok(Self {
            inner: Arc::new(RwLock::new(raft_log)),
        })
    }
}

impl<C> RaftLogReader<C> for WalLogStore<C>
where
    C: RaftTypeConfig,
    EntryOf<C>: Clone,
{
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<EntryOf<C>>, io::Error> {
        let (start, end) = range_boundary(range);

        let log = self.inner.read().await;

        let entries = log
            .read(start, end)
            .map(|res| res.map(|(_log_id, payload)| payload.0))
            .collect::<Result<Vec<_>, io::Error>>()?;

        Ok(entries)
    }

    async fn read_vote(&mut self) -> Result<Option<VoteOf<C>>, io::Error> {
        let log = self.inner.read().await;
        let vote = log.log_state().vote().map(|vote| vote.0.clone());

        Ok(vote)
    }
}

impl<C> RaftLogStorage<C> for WalLogStore<C>
where
    C: RaftTypeConfig,
    EntryOf<C>: Clone,
{
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<C>, io::Error> {
        let log = self.inner.read().await;
        let state = log.log_state();

        // `raft_log` advances `last` to the purged log id when a purge removes
        // every entry, so `last` never falls behind `purged` and needs no
        // fallback here.
        Ok(LogState {
            last_purged_log_id: state.purged().map(|log_id| log_id.0.clone()),
            last_log_id: state.last().map(|log_id| log_id.0.clone()),
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &VoteOf<C>) -> Result<(), io::Error> {
        let (tx, rx) = oneshot::channel();

        {
            let mut log = self.inner.write().await;

            log.save_vote(MsgPackVote(vote.clone()))?;
            log.flush(true, Some(Callback::Oneshot(tx)))?;
        }

        // A vote decides an election, so it must reach disk before this method
        // returns. The lock is released above so the fsync does not block other
        // access to the log.
        let flush_res = rx.await.map_err(io::Error::other)?;
        flush_res?;

        Ok(())
    }

    async fn save_committed(&mut self, committed: Option<LogIdOf<C>>) -> Result<(), io::Error> {
        let Some(committed) = committed else {
            return Ok(());
        };

        let mut log = self.inner.write().await;
        log.commit(MsgPack(committed))?;

        // No flush is requested here. openraft treats the committed log id as
        // an optimization, not as required durable state: losing it in a crash
        // only makes the state machine re-apply a few entries at startup. The
        // record waits in the write buffer and reaches disk with the next
        // `append`.
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogIdOf<C>>, io::Error> {
        let log = self.inner.read().await;
        let committed = log.log_state().committed().map(|log_id| log_id.0.clone());

        Ok(committed)
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<C>) -> Result<(), io::Error>
    where I: IntoIterator<Item = EntryOf<C>> + OptionalSend {
        let entries = entries.into_iter().map(|entry| {
            let log_id = entry.log_id();
            (MsgPack(log_id), MsgPack(entry))
        });

        let mut log = self.inner.write().await;

        log.append(entries)?;
        log.flush(true, Some(Callback::IOFlushed(callback)))?;

        // The flush runs on the `raft_log` worker thread, which calls the
        // callback when the entries are on disk.
        Ok(())
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogIdOf<C>>) -> Result<(), io::Error> {
        let truncate_at = last_log_id.next_index();

        let mut log = self.inner.write().await;

        let curr_last = log.log_state().last().map(|log_id| log_id.0.clone());
        if truncate_at >= curr_last.next_index() {
            tracing::debug!("log-wal: nothing to truncate at index {}", truncate_at);
            return Ok(());
        }

        log.truncate(truncate_at)?;

        // Truncation needs no flush. A crash before the record reaches disk
        // leaves the conflicting entries in place, and openraft truncates them
        // again after the restart.
        Ok(())
    }

    async fn purge(&mut self, log_id: LogIdOf<C>) -> Result<(), io::Error> {
        let mut log = self.inner.write().await;

        let curr_purged = log.log_state().purged().map(|log_id| log_id.0.clone());
        if log_id.index < curr_purged.next_index() {
            tracing::debug!("log-wal: already purged up to {:?}", curr_purged);
            return Ok(());
        }

        log.purge(MsgPack(log_id))?;

        // Losing the purge record itself costs nothing: openraft purges again
        // after the restart. The fsync is asked for on behalf of the chunk
        // files. `raft_log` unlinks a purged chunk only after a `flush(true, _)`
        // has put the covering purge record on disk, so without this call the
        // freed space waits for the next `append`. The flush is queued, not
        // awaited, so this method does not block on the fsync.
        log.flush(true, None)?;

        Ok(())
    }
}

/// Convert a range of log indexes into the `[start, end)` pair `raft_log` reads.
fn range_boundary<RB: RangeBounds<u64>>(range: RB) -> (u64, u64) {
    let start = match range.start_bound() {
        Bound::Included(&n) => n,
        Bound::Excluded(&n) => n + 1,
        Bound::Unbounded => 0,
    };

    let end = match range.end_bound() {
        Bound::Included(&n) => n + 1,
        Bound::Excluded(&n) => n,
        Bound::Unbounded => u64::MAX,
    };

    (start, end)
}
