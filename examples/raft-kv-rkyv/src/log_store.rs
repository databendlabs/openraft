use std::fmt::Debug;
use std::io;
use std::ops::RangeBounds;
use std::path::Path;
use std::sync::Arc;

use openraft::LogState;
use openraft::RaftLogReader;
use openraft::alias::VoteOf;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;
use tokio::sync::Mutex;

use crate::Entry;
use crate::LogId;
use crate::TypeConfig;
use crate::atomic_file::AtomicFile;
use crate::disk_state::DiskState;

/// Small reference store: optional whole-image persistence, not an optimized WAL.
#[derive(Clone, Default)]
pub(crate) struct LogStore {
    inner: Arc<Mutex<(DiskState, Option<AtomicFile>)>>,
}

impl LogStore {
    /// The caller must create and durably provision the directory before starting Raft.
    pub(crate) fn open(directory: &Path) -> io::Result<Self> {
        let disk = AtomicFile::open(directory)?;
        let state = match disk.read(DiskState::MAX_BYTES)? {
            Some(bytes) => DiskState::decode(&bytes)?,
            None => DiskState::default(),
        };
        Ok(Self {
            inner: Arc::new(Mutex::new((state, Some(disk)))),
        })
    }

    async fn read<T>(&self, read: impl FnOnce(&DiskState) -> T) -> io::Result<T> {
        let inner = self.inner.lock().await;
        if let Some(disk) = &inner.1 {
            disk.check()?;
        }
        Ok(read(&inner.0))
    }

    async fn update<F>(&self, update: F) -> io::Result<()>
    where F: FnOnce(&mut DiskState) + Send + 'static {
        let mut inner = self.inner.clone().lock_owned().await;
        if inner.1.is_none() {
            // Memory mode cannot fail and has no blocking I/O: mutate in place.
            update(&mut inner.0);
            return Ok(());
        }
        // Keep mutation and disk publication in the same blocking job. Cancellation of the
        // caller must not separate the in-memory image from the published one.
        // The owned async guard also keeps readers off runtime-blocking mutex waits.
        tokio::task::spawn_blocking(move || {
            let (state, disk) = &mut *inner;
            let disk = disk.as_mut().expect("persistent store");
            // The fence set by `publish` hides the in-place change until the image is durable.
            disk.publish(|| {
                update(state);
                state.encode()
            })
        })
        .await
        .map_err(io::Error::other)?
    }

    #[cfg(test)]
    pub(crate) async fn fail_at(&self, stage: &'static str) {
        self.inner.lock().await.1.as_mut().unwrap().fail_at = Some(stage);
    }
}

impl RaftLogReader<TypeConfig> for LogStore {
    async fn try_get_log_entries<R>(&mut self, range: R) -> io::Result<Vec<Entry>>
    where R: RangeBounds<u64> + Clone + Debug + Send {
        self.read(|state| state.entries.range(range).map(|(_, entry)| entry.clone()).collect()).await
    }

    async fn read_vote(&mut self) -> io::Result<Option<VoteOf<TypeConfig>>> {
        self.read(|state| state.vote).await
    }
}

impl RaftLogStorage<TypeConfig> for LogStore {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> io::Result<LogState<TypeConfig>> {
        self.read(|state| LogState {
            last_purged_log_id: state.purged,
            last_log_id: state.entries.last_key_value().map(|(_, entry)| entry.log_id).or(state.purged),
        })
        .await
    }

    async fn get_log_reader(&mut self) -> Self {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &VoteOf<TypeConfig>) -> io::Result<()> {
        let vote = *vote;
        self.update(move |state| state.vote = Some(vote)).await
    }

    async fn save_committed(&mut self, committed: Option<LogId>) -> io::Result<()> {
        self.update(move |state| state.committed = committed).await
    }

    async fn read_committed(&mut self) -> io::Result<Option<LogId>> {
        self.read(|state| state.committed).await
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<TypeConfig>) -> io::Result<()>
    where I: IntoIterator<Item = Entry> + Send {
        let entries: Vec<_> = entries.into_iter().collect();
        let result = self
            .update(move |state| {
                for entry in entries {
                    state.entries.insert(entry.log_id.index, entry);
                }
            })
            .await;
        callback.io_completed(result.as_ref().map(|_| ()).map_err(|e| io::Error::new(e.kind(), e.to_string())));
        result
    }

    async fn truncate_after(&mut self, last: Option<LogId>) -> io::Result<()> {
        self.update(move |state| state.entries.retain(|index, _| last.is_some_and(|last| *index <= last.index)))
            .await
    }

    async fn purge(&mut self, log_id: LogId) -> io::Result<()> {
        self.update(move |state| {
            debug_assert!(
                state.purged <= Some(log_id),
                "purge must not move backwards: purged {:?}, requested {}",
                state.purged,
                log_id
            );
            state.entries.retain(|index, _| *index > log_id.index);
            state.purged = Some(log_id);
        })
        .await
    }
}

#[cfg(test)]
mod log_store_test;
