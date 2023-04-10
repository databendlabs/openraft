use std::fmt::Debug;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ops::RangeBounds;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::sleep;

use crate::async_trait::async_trait;
use crate::defensive::DefensiveCheckBase;
use crate::display_ext::DisplaySlice;
use crate::storage::LogState;
use crate::storage::RaftLogReader;
use crate::storage::RaftSnapshotBuilder;
use crate::storage::Snapshot;
use crate::DefensiveCheck;
use crate::LogId;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;
use crate::StorageError;
use crate::StoredMembership;
use crate::Vote;
use crate::Wrapper;

#[derive(Clone, Debug)]
pub(crate) struct Config {
    /// Time in milliseconds to delay before reading logs.
    ///
    /// For debug only.
    delay_log_read: Arc<AtomicU64>,

    #[allow(dead_code)]
    id: u64,
}

impl Config {
    pub(crate) fn new() -> Self {
        static CONFIG_ID: AtomicU64 = AtomicU64::new(1);

        let id = CONFIG_ID.fetch_add(1, Ordering::Relaxed);

        Self {
            delay_log_read: Arc::new(AtomicU64::new(0)),
            id,
        }
    }

    pub(crate) fn get_delay_log_read(&self) -> Option<Duration> {
        let d = self.delay_log_read.load(Ordering::Relaxed);

        if d == 0 {
            return None;
        }
        Some(Duration::from_millis(d))
    }
}

/// Extended store backed by another impl.
///
/// It provides defensive check against input and the state of underlying store.
/// And it provides more APIs.
pub struct StoreExt<C: RaftTypeConfig, T: RaftStorage<C>> {
    // TODO: move defensive to config
    defensive: Arc<AtomicBool>,

    config: Config,

    inner: T,

    c: PhantomData<C>,
}

impl<C: RaftTypeConfig, T: RaftStorage<C> + Clone> Clone for StoreExt<C, T> {
    fn clone(&self) -> Self {
        Self {
            defensive: self.defensive.clone(),
            config: self.config.clone(),
            inner: self.inner.clone(),
            c: PhantomData,
        }
    }
}

impl<C: RaftTypeConfig, T: RaftStorage<C>> Deref for StoreExt<C, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<C: RaftTypeConfig, T: RaftStorage<C>> StoreExt<C, T> {
    /// Create a StoreExt backed by another store.
    pub fn new(inner: T) -> Self {
        StoreExt {
            defensive: Arc::new(AtomicBool::new(false)),
            config: Config::new(),
            inner,
            c: PhantomData,
        }
    }

    pub fn set_delay_log_read(&self, ms: u64) {
        self.config.delay_log_read.store(ms, Ordering::Relaxed);
        let delay = self.config.delay_log_read.load(Ordering::Relaxed);
        tracing::info!("Set log reading delay to {delay}");
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
impl<C: RaftTypeConfig, T: RaftStorage<C>> RaftStorage<C> for StoreExt<C, T>
where
    T: RaftStorage<C>,
    C: RaftTypeConfig,
{
    type SnapshotData = T::SnapshotData;

    type LogReader = LogReaderExt<C, T>;

    type SnapshotBuilder = SnapshotBuilderExt<C, T>;

    #[tracing::instrument(level = "trace", skip(self))]
    async fn save_vote(&mut self, vote: &Vote<C::NodeId>) -> Result<(), StorageError<C::NodeId>> {
        self.defensive_incremental_vote(vote).await?;
        self.inner().save_vote(vote).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn read_vote(&mut self) -> Result<Option<Vote<C::NodeId>>, StorageError<C::NodeId>> {
        self.inner().read_vote().await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn last_applied_state(
        &mut self,
    ) -> Result<(Option<LogId<C::NodeId>>, StoredMembership<C::NodeId, C::Node>), StorageError<C::NodeId>> {
        self.inner().last_applied_state().await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn delete_conflict_logs_since(&mut self, log_id: LogId<C::NodeId>) -> Result<(), StorageError<C::NodeId>> {
        self.defensive_delete_conflict_gt_last_applied(log_id).await?;
        self.inner().delete_conflict_logs_since(log_id).await
    }

    #[tracing::instrument(level = "trace", skip_all)]
    async fn purge_logs_upto(&mut self, log_id: LogId<C::NodeId>) -> Result<(), StorageError<C::NodeId>> {
        self.defensive_purge_applied_le_last_applied(log_id).await?;
        self.inner().purge_logs_upto(log_id).await
    }

    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn append_to_log<I>(&mut self, entries: I) -> Result<(), StorageError<C::NodeId>>
    where I: IntoIterator<Item = C::Entry> + Send {
        if self.is_defensive() {
            let entries_vec = entries.into_iter().collect::<Vec<_>>();
            tracing::debug!(
                "Defensive check: append_to_log: entries={}",
                DisplaySlice::<_>(&entries_vec)
            );

            self.defensive_nonempty_input(&entries_vec).await?;
            self.defensive_consecutive_input(&entries_vec).await?;
            self.defensive_append_log_index_is_last_plus_one(&entries_vec).await?;
            self.defensive_append_log_id_gt_last(&entries_vec).await?;

            self.inner().append_to_log(entries_vec).await
        } else {
            self.inner().append_to_log(entries).await
        }
    }

    #[tracing::instrument(level = "trace", skip(self, entries), fields(entries=display(DisplaySlice::<_>(entries))))]
    async fn apply_to_state_machine(&mut self, entries: &[C::Entry]) -> Result<Vec<C::R>, StorageError<C::NodeId>> {
        self.defensive_nonempty_input(entries).await?;
        self.defensive_apply_index_is_last_applied_plus_one(entries).await?;
        self.defensive_apply_log_id_gt_last(entries).await?;

        self.inner().apply_to_state_machine(entries).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&mut self) -> Result<Box<Self::SnapshotData>, StorageError<C::NodeId>> {
        self.inner().begin_receiving_snapshot().await
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<C::NodeId, C::Node>,
        snapshot: Box<Self::SnapshotData>,
    ) -> Result<(), StorageError<C::NodeId>> {
        self.inner().install_snapshot(meta, snapshot).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<C::NodeId, C::Node, Self::SnapshotData>>, StorageError<C::NodeId>> {
        self.inner().get_current_snapshot().await
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        LogReaderExt {
            defensive: self.defensive.clone(),
            config: self.config.clone(),
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
    ) -> Result<Vec<C::Entry>, StorageError<C::NodeId>> {
        if let Some(d) = self.config.get_delay_log_read() {
            sleep(d).await;
        }

        self.defensive_nonempty_range(range.clone())?;
        self.inner().try_get_log_entries(range).await
    }

    async fn get_log_state(&mut self) -> Result<LogState<C>, StorageError<C::NodeId>> {
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
    async fn build_snapshot(
        &mut self,
    ) -> Result<Snapshot<C::NodeId, C::Node, T::SnapshotData>, StorageError<C::NodeId>> {
        self.inner.build_snapshot().await
    }
}

/// Extended log reader backed by another impl.
///
/// It provides defensive check against input and the state of underlying log reader.
pub struct LogReaderExt<C: RaftTypeConfig, T: RaftStorage<C>> {
    defensive: Arc<AtomicBool>,
    config: Config,
    inner: T::LogReader,
}

#[async_trait]
impl<C: RaftTypeConfig, T: RaftStorage<C>> RaftLogReader<C> for LogReaderExt<C, T> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &mut self,
        range: RB,
    ) -> Result<Vec<C::Entry>, StorageError<C::NodeId>> {
        if let Some(d) = self.config.get_delay_log_read() {
            sleep(d).await;
        }

        self.defensive_nonempty_range(range.clone())?;
        self.inner.try_get_log_entries(range).await
    }

    async fn get_log_state(&mut self) -> Result<LogState<C>, StorageError<C::NodeId>> {
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
