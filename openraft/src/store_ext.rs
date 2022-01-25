use std::fmt::Debug;
use std::marker::PhantomData;
use std::ops::RangeBounds;
use std::sync::RwLock;

use crate::async_trait::async_trait;
use crate::raft::Entry;
use crate::storage::LogState;
use crate::storage::Snapshot;
use crate::summary::MessageSummary;
use crate::AppData;
use crate::AppDataResponse;
use crate::DefensiveCheck;
use crate::EffectiveMembership;
use crate::LogId;
use crate::RaftStorage;
use crate::RaftStorageDebug;
use crate::SnapshotMeta;
use crate::StateMachineChanges;
use crate::StorageError;
use crate::Vote;
use crate::Wrapper;

/// Extended store backed by another impl.
///
/// It provides defensive check against input and the state of underlying store.
/// And it provides more APIs.
pub struct StoreExt<D, R, T> {
    defensive: RwLock<bool>,
    inner: T,
    p: PhantomData<(D, R)>,
}

impl<D, R, T> StoreExt<D, R, T> {
    /// Create a StoreExt backed by another store.
    pub fn new(inner: T) -> Self {
        StoreExt {
            defensive: RwLock::new(false),
            inner,
            p: Default::default(),
        }
    }
}

impl<D, R, T> Wrapper<D, R, T> for StoreExt<D, R, T>
where
    D: AppData,
    R: AppDataResponse,
    T: RaftStorage<D, R>,
{
    fn inner(&self) -> &T {
        &self.inner
    }
}

impl<D, R, T> DefensiveCheck<D, R, T> for StoreExt<D, R, T>
where
    D: AppData,
    R: AppDataResponse,
    T: RaftStorage<D, R>,
{
    fn set_defensive(&self, d: bool) {
        let mut defensive_flag = self.defensive.write().unwrap();
        *defensive_flag = d;
    }

    fn is_defensive(&self) -> bool {
        *self.defensive.read().unwrap()
    }
}

#[async_trait]
impl<D, R, T, SM> RaftStorageDebug<SM> for StoreExt<D, R, T>
where
    T: RaftStorage<D, R> + RaftStorageDebug<SM>,
    D: AppData,
    R: AppDataResponse,
{
    async fn get_state_machine(&self) -> SM {
        self.inner().get_state_machine().await
    }
}

#[async_trait]
impl<D, R, T> RaftStorage<D, R> for StoreExt<D, R, T>
where
    T: RaftStorage<D, R>,
    D: AppData,
    R: AppDataResponse,
{
    type SnapshotData = T::SnapshotData;

    #[tracing::instrument(level = "trace", skip(self))]
    async fn save_vote(&self, vote: &Vote) -> Result<(), StorageError> {
        self.defensive_incremental_vote(vote).await?;
        self.inner().save_vote(vote).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn read_vote(&self) -> Result<Option<Vote>, StorageError> {
        self.inner().read_vote().await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &self,
        range: RB,
    ) -> Result<Vec<Entry<D>>, StorageError> {
        self.defensive_nonempty_range(range.clone()).await?;

        self.inner().try_get_log_entries(range).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn last_applied_state(&self) -> Result<(Option<LogId>, Option<EffectiveMembership>), StorageError> {
        self.inner().last_applied_state().await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn delete_conflict_logs_since(&self, log_id: LogId) -> Result<(), StorageError> {
        self.defensive_delete_conflict_gt_last_applied(log_id).await?;
        self.inner().delete_conflict_logs_since(log_id).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn purge_logs_upto(&self, log_id: LogId) -> Result<(), StorageError> {
        self.defensive_purge_applied_le_last_applied(log_id).await?;
        self.inner().purge_logs_upto(log_id).await
    }

    #[tracing::instrument(level = "trace", skip(self, entries), fields(entries=%entries.summary()))]
    async fn append_to_log(&self, entries: &[&Entry<D>]) -> Result<(), StorageError> {
        self.defensive_nonempty_input(entries).await?;
        self.defensive_consecutive_input(entries).await?;
        self.defensive_append_log_index_is_last_plus_one(entries).await?;
        self.defensive_append_log_id_gt_last(entries).await?;

        self.inner().append_to_log(entries).await
    }

    #[tracing::instrument(level = "trace", skip(self, entries), fields(entries=%entries.summary()))]
    async fn apply_to_state_machine(&self, entries: &[&Entry<D>]) -> Result<Vec<R>, StorageError> {
        self.defensive_nonempty_input(entries).await?;
        self.defensive_apply_index_is_last_applied_plus_one(entries).await?;
        self.defensive_apply_log_id_gt_last(entries).await?;

        self.inner().apply_to_state_machine(entries).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&self) -> Result<Snapshot<Self::SnapshotData>, StorageError> {
        self.inner().build_snapshot().await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&self) -> Result<Box<Self::SnapshotData>, StorageError> {
        self.inner().begin_receiving_snapshot().await
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(
        &self,
        meta: &SnapshotMeta,
        snapshot: Box<Self::SnapshotData>,
    ) -> Result<StateMachineChanges, StorageError> {
        self.inner().install_snapshot(meta, snapshot).await
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&self) -> Result<Option<Snapshot<Self::SnapshotData>>, StorageError> {
        self.inner().get_current_snapshot().await
    }

    async fn get_log_state(&self) -> Result<LogState, StorageError> {
        self.defensive_no_dirty_log().await?;
        self.inner().get_log_state().await
    }
}
