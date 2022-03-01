use std::fmt::Debug;
use std::marker::PhantomData;
use std::ops::RangeBounds;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::async_trait::async_trait;
use crate::defensive::DefensiveCheckBase;
use crate::raft::Entry;
use crate::storage::LogState;
use crate::storage::RaftLogReader;
use crate::storage::RaftSnapshotBuilder;
use crate::storage::Snapshot;
use crate::summary::MessageSummary;
use crate::DefensiveCheck;
use crate::EffectiveMembership;
use crate::LogId;
use crate::RaftStorage;
use crate::RaftStorageDebug;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;
use crate::StateMachineChanges;
use crate::StorageError;
use crate::Vote;
use crate::Wrapper;

/// Extended store backed by another impl.
///
/// It provides defensive check against input and the state of underlying store.
/// And it provides more APIs.
pub struct StoreExt<C: RaftTypeConfig, T: RaftStorage<C>> {
    defensive: Arc<AtomicBool>,
    inner: T,
    c: PhantomData<C>,
}

impl<C: RaftTypeConfig, T: RaftStorage<C> + Clone> Clone for StoreExt<C, T> {
    fn clone(&self) -> Self {
        Self {
            defensive: self.defensive.clone(),
            inner: self.inner.clone(),
            c: PhantomData,
        }
    }
}

impl<C: RaftTypeConfig, T: RaftStorage<C>> StoreExt<C, T> {
    /// Create a StoreExt backed by another store.
    pub fn new(inner: T) -> Self {
        StoreExt {
            defensive: Arc::new(AtomicBool::new(false)),
            inner,
            c: PhantomData,
        }
    }
}

impl<C: RaftTypeConfig, T: RaftStorage<C>> Wrapper<C, T> for StoreExt<C, T>
where
    C: RaftTypeConfig,
    T: RaftStorage<C>,
{
    fn inner(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<C: RaftTypeConfig, T: RaftStorage<C>> DefensiveCheckBase<C> for StoreExt<C, T>
where
    C: RaftTypeConfig,
    T: RaftStorage<C>,
{
    fn set_defensive(&self, d: bool) {
        self.defensive.store(d, Ordering::Relaxed);
    }

    fn is_defensive(&self) -> bool {
        self.defensive.load(Ordering::Relaxed)
    }
}

impl<C: RaftTypeConfig, T: RaftStorage<C>> DefensiveCheck<C, T> for StoreExt<C, T>
where
    C: RaftTypeConfig,
    T: RaftStorage<C>,
{
}

#[async_trait]
impl<C, T, SM> RaftStorageDebug<SM> for StoreExt<C, T>
where
    T: RaftStorage<C> + RaftStorageDebug<SM>,
    C: RaftTypeConfig,
{
    async fn get_state_machine(&mut self) -> SM {
        self.inner().get_state_machine().await
    }
}

#[async_trait]
impl<C: RaftTypeConfig, T: RaftStorage<C>> RaftStorage<C> for StoreExt<C, T>
where
    T: RaftStorage<C>,
    C: RaftTypeConfig,
{
    type SnapshotData = T::SnapshotData;

    type LogReader = LogReaderExt<C, T>;

    type SnapshotBuilder = SnapshotBuilderExt<C, T>;

    #[tracing::instrument(level = "trace", skip(self))]
    async fn save_vote(&mut self, vote: &Vote<C>) -> Result<(), StorageError<C>> {
        self.defensive_incremental_vote(vote).await?;
        self.inner().save_vote(vote).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn read_vote(&mut self) -> Result<Option<Vote<C>>, StorageError<C>> {
        self.inner().read_vote().await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn last_applied_state(
        &mut self,
    ) -> Result<(Option<LogId<C>>, Option<EffectiveMembership<C>>), StorageError<C>> {
        self.inner().last_applied_state().await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn delete_conflict_logs_since(&mut self, log_id: LogId<C>) -> Result<(), StorageError<C>> {
        self.defensive_delete_conflict_gt_last_applied(log_id).await?;
        self.inner().delete_conflict_logs_since(log_id).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn purge_logs_upto(&mut self, log_id: LogId<C>) -> Result<(), StorageError<C>> {
        self.defensive_purge_applied_le_last_applied(log_id).await?;
        self.inner().purge_logs_upto(log_id).await
    }

    #[tracing::instrument(level = "trace", skip(self, entries), fields(entries=%entries.summary()))]
    async fn append_to_log(&mut self, entries: &[&Entry<C>]) -> Result<(), StorageError<C>> {
        self.defensive_nonempty_input(entries).await?;
        self.defensive_consecutive_input(entries).await?;
        self.defensive_append_log_index_is_last_plus_one(entries).await?;
        self.defensive_append_log_id_gt_last(entries).await?;

        self.inner().append_to_log(entries).await
    }

    #[tracing::instrument(level = "trace", skip(self, entries), fields(entries=%entries.summary()))]
    async fn apply_to_state_machine(&mut self, entries: &[&Entry<C>]) -> Result<Vec<C::R>, StorageError<C>> {
        self.defensive_nonempty_input(entries).await?;
        self.defensive_apply_index_is_last_applied_plus_one(entries).await?;
        self.defensive_apply_log_id_gt_last(entries).await?;

        self.inner().apply_to_state_machine(entries).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&mut self) -> Result<Box<Self::SnapshotData>, StorageError<C>> {
        self.inner().begin_receiving_snapshot().await
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<C>,
        snapshot: Box<Self::SnapshotData>,
    ) -> Result<StateMachineChanges<C>, StorageError<C>> {
        self.inner().install_snapshot(meta, snapshot).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<C, Self::SnapshotData>>, StorageError<C>> {
        self.inner().get_current_snapshot().await
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        LogReaderExt {
            defensive: self.defensive.clone(),
            inner: self.inner().get_log_reader().await,
        }
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        SnapshotBuilderExt {
            inner: self.inner().get_snapshot_builder().await,
        }
    }
}

#[async_trait]
impl<C: RaftTypeConfig, T: RaftStorage<C>> RaftLogReader<C> for StoreExt<C, T> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<C>>, StorageError<C>> {
        self.defensive_nonempty_range(range.clone())?;
        self.inner().try_get_log_entries(range).await
    }

    async fn get_log_state(&mut self) -> Result<LogState<C>, StorageError<C>> {
        self.defensive_no_dirty_log().await?;
        self.inner().get_log_state().await
    }
}

/// Extended snapshot builder backed by another impl.
///
/// It provides defensive check against input and the state of underlying snapshot builder.
pub struct SnapshotBuilderExt<C: RaftTypeConfig, T: RaftStorage<C>> {
    inner: T::SnapshotBuilder,
}

#[async_trait]
impl<C: RaftTypeConfig, T: RaftStorage<C>> RaftSnapshotBuilder<C, T::SnapshotData> for SnapshotBuilderExt<C, T> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot<C, T::SnapshotData>, StorageError<C>> {
        self.inner.build_snapshot().await
    }
}

/// Extended log reader backed by another impl.
///
/// It provides defensive check against input and the state of underlying log reader.
pub struct LogReaderExt<C: RaftTypeConfig, T: RaftStorage<C>> {
    defensive: Arc<AtomicBool>,
    inner: T::LogReader,
}

#[async_trait]
impl<C: RaftTypeConfig, T: RaftStorage<C>> RaftLogReader<C> for LogReaderExt<C, T> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<C>>, StorageError<C>> {
        self.defensive_nonempty_range(range.clone())?;
        self.inner.try_get_log_entries(range).await
    }

    async fn get_log_state(&mut self) -> Result<LogState<C>, StorageError<C>> {
        // TODO self.defensive_no_dirty_log().await?;
        // Log state via LogReader is requested exactly at one place in the replication loop.
        // Find a way how to either remove it there or assert here properly.
        self.inner.get_log_state().await
    }
}

impl<C: RaftTypeConfig, T: RaftStorage<C>> DefensiveCheckBase<C> for LogReaderExt<C, T>
where
    C: RaftTypeConfig,
    T: RaftStorage<C>,
{
    fn set_defensive(&self, d: bool) {
        self.defensive.store(d, Ordering::Relaxed);
    }

    fn is_defensive(&self) -> bool {
        self.defensive.load(Ordering::Relaxed)
    }
}
