//! A state machine whose every `apply()` call takes a fixed time longer.

use std::io;
use std::time::Duration;

use futures::Stream;
use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::alias::LogIdOf;
use openraft::alias::SnapshotMetaOf;
use openraft::alias::SnapshotOf;
use openraft::alias::StoredMembershipOf;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;

/// Delay every `apply()` call of `inner` by `delay`, like a store that makes each applied batch
/// durable.
///
/// The delay occupies a blocking-pool thread before the call applies its entries, so the
/// responses of those entries wait for it as well. A blocking sleep keeps sub-millisecond delays
/// accurate; the Tokio timer would round them up to its 1 ms tick.
#[derive(Clone)]
pub(crate) struct SlowApply<SM> {
    inner: SM,
    delay: Duration,
}

impl<SM> SlowApply<SM> {
    pub(crate) fn new(inner: SM, delay: Duration) -> Self {
        Self { inner, delay }
    }
}

impl<C, SM> RaftStateMachine<C> for SlowApply<SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    type SnapshotData = SM::SnapshotData;
    type SnapshotBuilder = SM::SnapshotBuilder;

    async fn applied_state(&mut self) -> Result<(Option<LogIdOf<C>>, StoredMembershipOf<C>), io::Error> {
        self.inner.applied_state().await
    }

    async fn apply<Strm>(&mut self, entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<C>, io::Error>> + Unpin + OptionalSend {
        if !self.delay.is_zero() {
            let delay = self.delay;
            let sleep = tokio::task::spawn_blocking(move || std::thread::sleep(delay));
            sleep.await.map_err(io::Error::other)?;
        }
        self.inner.apply(entries).await
    }

    async fn try_create_snapshot_builder(&mut self, force: bool) -> Option<Self::SnapshotBuilder> {
        self.inner.try_create_snapshot_builder(force).await
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.inner.get_snapshot_builder().await
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMetaOf<C>,
        snapshot: Self::SnapshotData,
    ) -> Result<(), io::Error> {
        self.inner.install_snapshot(meta, snapshot).await
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<SnapshotOf<C, Self::SnapshotData>>, io::Error> {
        self.inner.get_current_snapshot().await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use openraft::testing::blank_ent;
    use openraft::testing::log_id;
    use raft_kv_log_wal_sm_mem::StateMachineStore;
    use raft_kv_log_wal_sm_mem::TypeConfig;

    use super::*;

    #[tokio::test]
    async fn an_apply_call_takes_the_delay_and_still_applies_its_entries() -> io::Result<()> {
        let delay = Duration::from_millis(20);
        let mut machine = SlowApply::new(StateMachineStore::default(), delay);
        let entries = [Ok((blank_ent::<TypeConfig>(1, 1, 3), None))];

        let started = Instant::now();
        machine.apply(futures::stream::iter(entries)).await?;
        let elapsed = started.elapsed();

        assert!(elapsed >= delay, "apply() returned after {elapsed:?}");
        let (applied, _) = machine.applied_state().await?;
        assert_eq!(Some(log_id::<TypeConfig>(1, 1, 3)), applied);
        Ok(())
    }
}
