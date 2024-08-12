use std::collections::BTreeSet;
use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::ops::RangeBounds;
use std::time::Duration;

use maplit::btreeset;

use crate::async_runtime::MpscUnboundedReceiver;
use crate::async_runtime::MpscUnboundedSender;
use crate::core::notification::Notification;
use crate::entry::RaftEntry;
use crate::log_id::RaftLogId;
use crate::membership::EffectiveMembership;
use crate::raft_state::io_state::io_id::IOId;
use crate::raft_state::LogStateReader;
use crate::raft_state::RaftState;
use crate::storage::IOFlushed;
use crate::storage::LogState;
use crate::storage::RaftLogReaderExt;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::storage::StorageHelper;
use crate::testing::log::StoreBuilder;
use crate::type_config::TypeConfigExt;
use crate::vote::CommittedLeaderId;
use crate::LogId;
use crate::Membership;
use crate::NodeId;
use crate::OptionalSend;
use crate::RaftLogReader;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StoredMembership;
use crate::Vote;

const NODE_ID: u64 = 0;

/// Helper to construct a `BTreeSet` of `C::NodeId` from numbers.
macro_rules! btreeset {
    ($($key:expr,)+) => (btreeset!($($key),+));
    ( $($key:expr),* ) => {{
        let mut _set = ::std::collections::BTreeSet::new();
        $( _set.insert($key.into()); )*
        _set
    }};
}

/// Allows [`RaftLogStorage`] to access methods provided by [`RaftLogReader`] in this test.
trait ReaderExt<C>: RaftLogStorage<C>
where C: RaftTypeConfig
{
    /// Proxy method to invoke [`RaftLogReaderExt::get_log_id`].
    async fn get_log_id(&mut self, log_index: u64) -> Result<LogId<C::NodeId>, StorageError<C>> {
        self.get_log_reader().await.get_log_id(log_index).await
    }

    /// Proxy method to invoke [`RaftLogReaderExt::try_get_log_entry`].
    async fn try_get_log_entry(&mut self, log_index: u64) -> Result<Option<C::Entry>, StorageError<C>> {
        self.get_log_reader().await.try_get_log_entry(log_index).await
    }

    /// Proxy method to invoke [`RaftLogReader::try_get_log_entries`].
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<C::Entry>, StorageError<C>> {
        self.get_log_reader().await.try_get_log_entries(range).await
    }

    /// Proxy method to invoke [`RaftLogReader::read_vote`].
    async fn read_vote(&mut self) -> Result<Option<Vote<C::NodeId>>, StorageError<C>> {
        self.get_log_reader().await.read_vote().await
    }

    /// Proxy method to invoke [`RaftLogReader::limited_get_log_entries`].
    async fn limited_get_log_entries(&mut self, start: u64, end: u64) -> Result<Vec<C::Entry>, StorageError<C>> {
        self.get_log_reader().await.limited_get_log_entries(start, end).await
    }
}

impl<C, S> ReaderExt<C> for S
where
    C: RaftTypeConfig,
    S: RaftLogStorage<C>,
{
}

/// Test suite to ensure a `RaftStore` impl works as expected.
///
/// Additional traits are required to be implemented by the store builder for testing:
/// - `C::D` and `C::R` requires `Debug` for debugging.
/// - `C::NodeId` requires `From<u64>` to build a node id.
pub struct Suite<C, LS, SM, B, G>
where
    C: RaftTypeConfig,
    C::D: Debug,
    C::R: Debug,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
    B: StoreBuilder<C, LS, SM, G>,
    G: Send + Sync,
{
    _p: PhantomData<(C, LS, SM, B, G)>,
}

#[allow(unused)]
impl<C, LS, SM, B, G> Suite<C, LS, SM, B, G>
where
    C: RaftTypeConfig,
    C::D: Debug,
    C::R: Debug,
    C::NodeId: From<u64>,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
    B: StoreBuilder<C, LS, SM, G>,
    G: Send + Sync,
{
    pub async fn test_all(builder: B) -> Result<(), StorageError<C>> {
        Suite::test_store(&builder).await?;
        Ok(())
    }

    pub async fn test_store(builder: &B) -> Result<(), StorageError<C>> {
        run_test(builder, Self::last_membership_in_log_initial).await?;
        run_test(builder, Self::last_membership_in_log).await?;
        run_test(builder, Self::last_membership_in_log_multi_step).await?;
        run_test(builder, Self::get_membership_initial).await?;
        run_test(builder, Self::get_membership_from_log_and_empty_sm).await?;
        run_test(builder, Self::get_membership_from_empty_log_and_sm).await?;
        run_test(builder, Self::get_membership_from_log_le_sm_last_applied).await?;
        run_test(builder, Self::get_membership_from_log_gt_sm_last_applied_1).await?;
        run_test(builder, Self::get_membership_from_log_gt_sm_last_applied_2).await?;
        run_test(builder, Self::get_initial_state_without_init).await?;
        run_test(builder, Self::get_initial_state_membership_from_log_and_sm).await?;
        run_test(builder, Self::get_initial_state_with_state).await?;
        run_test(builder, Self::get_initial_state_last_log_gt_sm).await?;
        run_test(builder, Self::get_initial_state_last_log_lt_sm).await?;
        run_test(builder, Self::get_initial_state_log_ids).await?;
        run_test(builder, Self::get_initial_state_re_apply_committed).await?;
        run_test(builder, Self::save_vote).await?;
        run_test(builder, Self::get_log_entries).await?;
        run_test(builder, Self::limited_get_log_entries).await?;
        run_test(builder, Self::try_get_log_entry).await?;
        run_test(builder, Self::initial_logs).await?;
        run_test(builder, Self::get_log_state).await?;
        run_test(builder, Self::get_log_id).await?;
        run_test(builder, Self::last_id_in_log).await?;
        run_test(builder, Self::last_applied_state).await?;
        run_test(builder, Self::purge_logs_upto_0).await?;
        run_test(builder, Self::purge_logs_upto_5).await?;
        run_test(builder, Self::purge_logs_upto_20).await?;
        run_test(builder, Self::delete_logs_since_11).await?;
        run_test(builder, Self::delete_logs_since_0).await?;
        run_test(builder, Self::append_to_log).await?;
        run_test(builder, Self::snapshot_meta).await?;

        run_test(builder, Self::apply_single).await?;
        run_test(builder, Self::apply_multiple).await?;

        Self::transfer_snapshot(builder).await?;

        // TODO(xp): test: do_log_compaction

        Ok(())
    }

    pub async fn last_membership_in_log_initial(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        let membership = StorageHelper::new(&mut store, &mut sm).last_membership_in_log(0).await?;

        assert!(membership.is_empty());

        Ok(())
    }

    pub async fn last_membership_in_log(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        tracing::info!("--- no log, do not read membership from state machine");
        {
            apply(&mut sm, [
                blank_ent_0::<C>(1, 1),
                membership_ent_0::<C>(1, 2, btreeset! {3,4,5}),
            ])
            .await?;

            let mem = StorageHelper::new(&mut store, &mut sm).last_membership_in_log(0).await?;

            assert!(mem.is_empty());
        }

        // Ensure last_membership_in_log() won't be affected by state machine.
        tracing::info!("--- membership presents in log, smaller than last_applied, read from log");
        {
            append(&mut store, [membership_ent_0::<C>(1, 1, btreeset! {1,2,3})]).await?;

            let mem = StorageHelper::new(&mut store, &mut sm).last_membership_in_log(0).await?;
            assert_eq!(1, mem.len());
            let mem = mem[0].clone();
            assert_eq!(&Membership::new(vec![btreeset! {1, 2, 3}], None), mem.membership(),);

            let mem = StorageHelper::new(&mut store, &mut sm).last_membership_in_log(1).await?;
            assert_eq!(1, mem.len());
            let mem = mem[0].clone();
            assert_eq!(&Membership::new(vec![btreeset! {1, 2, 3}], None), mem.membership(),);

            let mem = StorageHelper::new(&mut store, &mut sm).last_membership_in_log(2).await?;
            assert!(mem.is_empty());
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read 2 membership entries from log");
        {
            append(&mut store, [
                blank_ent_0::<C>(1, 2),
                membership_ent_0::<C>(1, 3, btreeset! {7,8,9}),
                blank_ent_0::<C>(1, 4),
            ])
            .await?;

            let mems = StorageHelper::new(&mut store, &mut sm).last_membership_in_log(0).await?;
            assert_eq!(2, mems.len());

            let mem = mems[0].clone();
            assert_eq!(&Membership::new(vec![btreeset! {1,2,3}], None), mem.membership(),);

            let mem = mems[1].clone();
            assert_eq!(&Membership::new(vec![btreeset! {7,8,9}], None), mem.membership(),);
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read from log but since_index is greater than the last");
        {
            let mem = StorageHelper::new(&mut store, &mut sm).last_membership_in_log(4).await?;
            assert!(mem.is_empty());
        }

        tracing::info!("--- 3 memberships in log, only return the last 2 of them");
        {
            append(&mut store, [membership_ent_0::<C>(1, 5, btreeset! {10,11})]).await?;

            let mems = StorageHelper::new(&mut store, &mut sm).last_membership_in_log(0).await?;
            assert_eq!(2, mems.len());

            let mem = mems[0].clone();
            assert_eq!(&Membership::new(vec![btreeset! {7,8,9}], None), mem.membership(),);

            let mem = mems[1].clone();
            assert_eq!(&Membership::new(vec![btreeset! {10,11}], None), mem.membership(),);
        }

        Ok(())
    }

    pub async fn last_membership_in_log_multi_step(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        tracing::info!("--- find membership log entry backwards, multiple steps");
        {
            append(&mut store, [
                //
                membership_ent_0::<C>(1, 1, btreeset! {1,2,3}),
                membership_ent_0::<C>(1, 2, btreeset! {3,4,5}),
            ])
            .await?;

            for i in 3..100 {
                append(&mut store, [blank_ent_0::<C>(1, i)]).await?;
            }

            append(&mut store, [membership_ent_0::<C>(1, 100, btreeset! {5,6,7})]).await?;

            let mems = StorageHelper::new(&mut store, &mut sm).last_membership_in_log(0).await?;
            assert_eq!(2, mems.len());
            let mem = mems[0].clone();
            assert_eq!(&Membership::new(vec![btreeset! {3,4,5}], None), mem.membership(),);

            let mem = mems[1].clone();
            assert_eq!(&Membership::new(vec![btreeset! {5,6,7}], None), mem.membership(),);
        }

        Ok(())
    }

    pub async fn get_membership_initial(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        let mem_state = StorageHelper::new(&mut store, &mut sm).get_membership().await?;

        assert_eq!(&EffectiveMembership::default(), mem_state.committed().as_ref());
        assert_eq!(&EffectiveMembership::default(), mem_state.effective().as_ref());

        Ok(())
    }

    pub async fn get_membership_from_log_and_empty_sm(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        tracing::info!("--- no log, read membership from state machine");
        {
            // There is an empty membership config in an empty state machine.

            append(&mut store, [membership_ent_0::<C>(1, 1, btreeset! {1,2,3})]).await?;

            let mem_state = StorageHelper::new(&mut store, &mut sm).get_membership().await?;

            assert_eq!(&EffectiveMembership::default(), mem_state.committed().as_ref());
            assert_eq!(
                &Membership::new(vec![btreeset! {1,2,3}], None),
                mem_state.effective().membership(),
            );
        }

        Ok(())
    }

    pub async fn get_membership_from_empty_log_and_sm(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        tracing::info!("--- no log, read membership from state machine");
        {
            apply(&mut sm, [
                blank_ent_0::<C>(1, 1),
                membership_ent_0::<C>(1, 2, btreeset! {3,4,5}),
            ])
            .await?;

            let mem_state = StorageHelper::new(&mut store, &mut sm).get_membership().await?;

            assert_eq!(
                &Membership::new(vec![btreeset! {3,4,5}], None),
                mem_state.committed().membership(),
            );
            assert_eq!(
                &Membership::new(vec![btreeset! {3,4,5}], None),
                mem_state.effective().membership(),
            );
        }
        Ok(())
    }

    pub async fn get_membership_from_log_le_sm_last_applied(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        tracing::info!("--- membership presents in log, but smaller than last_applied, read from state machine");
        {
            apply(&mut sm, [
                blank_ent_0::<C>(1, 1),
                membership_ent_0::<C>(1, 2, btreeset! {3,4,5}),
                blank_ent_0::<C>(1, 3),
                blank_ent_0::<C>(1, 4),
            ])
            .await?;

            // Intentionally append a membership entry that does not match the state machine,
            // in order to see which membership is loaded.
            append(&mut store, [
                blank_ent_0::<C>(1, 1),
                blank_ent_0::<C>(1, 2),
                membership_ent_0::<C>(1, 3, btreeset! {1,2,3}),
            ])
            .await?;

            let mem_state = StorageHelper::new(&mut store, &mut sm).get_membership().await?;

            assert_eq!(
                &Membership::new(vec![btreeset! {3,4,5}], None),
                mem_state.committed().membership(),
            );
            assert_eq!(
                &Membership::new(vec![btreeset! {3,4,5}], None),
                mem_state.effective().membership(),
            );
        }
        Ok(())
    }

    pub async fn get_membership_from_log_gt_sm_last_applied_1(
        mut store: LS,
        mut sm: SM,
    ) -> Result<(), StorageError<C>> {
        tracing::info!("--- membership presents in log and > sm.last_applied, read from log");
        {
            apply(&mut sm, [
                blank_ent_0::<C>(1, 1),
                membership_ent_0::<C>(1, 2, btreeset! {3,4,5}),
            ])
            .await?;

            append(&mut store, [
                membership_ent_0::<C>(1, 1, btreeset! {1,2,3}),
                blank_ent_0::<C>(1, 2),
                membership_ent_0::<C>(1, 3, btreeset! {7,8,9}),
            ])
            .await?;

            let mem_state = StorageHelper::new(&mut store, &mut sm).get_membership().await?;

            assert_eq!(
                &Membership::new(vec![btreeset! {3,4,5}], None),
                mem_state.committed().membership(),
            );
            assert_eq!(
                &Membership::new(vec![btreeset! {7,8,9}], None),
                mem_state.effective().membership(),
            );
        }
        Ok(())
    }

    pub async fn get_membership_from_log_gt_sm_last_applied_2(
        mut store: LS,
        mut sm: SM,
    ) -> Result<(), StorageError<C>> {
        tracing::info!("--- two membership present in log and > sm.last_applied, read 2 from log");
        {
            apply(&mut sm, [
                blank_ent_0::<C>(1, 1),
                membership_ent_0::<C>(1, 2, btreeset! {3,4,5}),
            ])
            .await?;

            append(&mut store, [
                membership_ent_0::<C>(1, 1, btreeset! {1,2,3}),
                blank_ent_0::<C>(1, 2),
                membership_ent_0::<C>(1, 3, btreeset! {7,8,9}),
                blank_ent_0::<C>(1, 4),
                membership_ent_0::<C>(1, 5, btreeset! {10,11}),
            ])
            .await?;

            let mem_state = StorageHelper::new(&mut store, &mut sm).get_membership().await?;

            assert_eq!(
                &Membership::new(vec![btreeset! {7,8,9}], None),
                mem_state.committed().membership(),
            );
            assert_eq!(
                &Membership::new(vec![btreeset! {10,11}], None),
                mem_state.effective().membership(),
            );
        }

        Ok(())
    }

    pub async fn get_initial_state_without_init(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;
        let mut want = RaftState::<C>::default();
        want.vote.update(
            initial.vote.last_update().unwrap(),
            Duration::default(),
            Vote::default(),
        );
        want.io_state.io_progress.accept(IOId::new(Vote::default()));
        want.io_state.io_progress.submit(IOId::new(Vote::default()));
        want.io_state.io_progress.flush(IOId::new(Vote::default()));

        assert_eq!(want, initial, "uninitialized state");
        Ok(())
    }

    pub async fn get_initial_state_with_state(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        Self::default_vote(&mut store).await?;

        append(&mut store, [
            blank_ent_0::<C>(0, 0),
            blank_ent_0::<C>(1, 1),
            blank_ent_0::<C>(3, 2),
        ])
        .await?;

        apply(&mut sm, [blank_ent_0::<C>(3, 1)]).await?;

        let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;

        assert_eq!(
            Some(&log_id_0(3, 2)),
            initial.last_log_id(),
            "state machine has higher log"
        );
        assert_eq!(
            initial.committed(),
            Some(&log_id_0(3, 1)),
            "unexpected value for last applied log"
        );
        assert_eq!(
            Vote::new(1, NODE_ID.into()),
            *initial.vote_ref(),
            "unexpected value for default hard state"
        );
        Ok(())
    }

    pub async fn get_initial_state_membership_from_log_and_sm(
        mut store: LS,
        mut sm: SM,
    ) -> Result<(), StorageError<C>> {
        // It should never return membership from logs that are included in state machine present.

        Self::default_vote(&mut store).await?;

        // copy the test from get_membership_config

        tracing::info!("--- no log, read membership from state machine");
        {
            apply(&mut sm, [
                blank_ent_0::<C>(1, 1),
                membership_ent_0::<C>(1, 2, btreeset! {3,4,5}),
            ])
            .await?;

            let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;

            assert_eq!(
                &Membership::new(vec![btreeset! {3,4,5}], None),
                initial.membership_state.effective().membership(),
            );
        }

        tracing::info!("--- membership presents in log, but smaller than last_applied, read from state machine");
        {
            append(&mut store, [membership_ent_0::<C>(1, 1, btreeset! {1,2,3})]).await?;

            let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;

            assert_eq!(
                &Membership::new(vec![btreeset! {3,4,5}], None),
                initial.membership_state.effective().membership(),
            );
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read from log");
        {
            append(&mut store, [membership_ent_0::<C>(1, 3, btreeset! {1,2,3})]).await?;

            let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;

            assert_eq!(
                &Membership::new(vec![btreeset! {1,2,3}], None),
                initial.membership_state.effective().membership(),
            );
        }

        Ok(())
    }

    pub async fn get_initial_state_last_log_gt_sm(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        Self::default_vote(&mut store).await?;

        append(&mut store, [blank_ent_0::<C>(0, 0), blank_ent_0::<C>(2, 1)]).await?;

        apply(&mut sm, [blank_ent_0::<C>(1, 1), blank_ent_0::<C>(1, 2)]).await?;

        let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;

        assert_eq!(
            Some(&log_id_0(2, 1)),
            initial.last_log_id(),
            "state machine has higher log"
        );
        Ok(())
    }

    pub async fn get_initial_state_last_log_lt_sm(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        Self::default_vote(&mut store).await?;

        append(&mut store, [blank_ent_0::<C>(1, 2)]).await?;

        apply(&mut sm, [blank_ent_0::<C>(3, 1)]).await?;

        let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;

        assert_eq!(
            Some(&log_id_0(3, 1)),
            initial.last_log_id(),
            "state machine has higher log"
        );
        assert_eq!(
            initial.last_purged_log_id().copied(),
            Some(log_id_0(3, 1)),
            "state machine has higher log"
        );
        Ok(())
    }

    pub async fn get_initial_state_log_ids(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        let log_id = |t, n: u64, i| LogId::<C::NodeId> {
            leader_id: CommittedLeaderId::new(t, n.into()),
            index: i,
        };

        tracing::info!("--- empty store, expect []");
        {
            let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;
            assert_eq!(Vec::<LogId<C::NodeId>>::new(), initial.log_ids.key_log_ids());
        }

        tracing::info!("--- log terms: [0], last_purged_log_id is None, expect [(0,0)]");
        {
            append(&mut store, [blank_ent_0::<C>(0, 0)]).await?;

            let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;
            assert_eq!(vec![log_id(0, 0, 0)], initial.log_ids.key_log_ids());
        }

        tracing::info!("--- log terms: [0,1,1,2], last_purged_log_id is None, expect [(0,0),(1,1),(2,3)]");
        {
            append(&mut store, [
                blank_ent_0::<C>(1, 1),
                blank_ent_0::<C>(1, 2),
                blank_ent_0::<C>(2, 3),
            ])
            .await?;

            let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;
            assert_eq!(
                vec![log_id(0, 0, 0), log_id(1, 0, 1), log_id(2, 0, 3)],
                initial.log_ids.key_log_ids()
            );
        }

        tracing::info!(
            "--- log terms: [0,1,1,2,2,3,3], last_purged_log_id is None, expect [(0,0),(1,1),(2,3),(3,5),(3,6)]"
        );
        {
            append(&mut store, [
                blank_ent_0::<C>(2, 4),
                blank_ent_0::<C>(3, 5),
                blank_ent_0::<C>(3, 6),
            ])
            .await?;

            let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;
            assert_eq!(
                vec![
                    log_id(0, 0, 0),
                    log_id(1, 0, 1),
                    log_id(2, 0, 3),
                    log_id(3, 0, 5),
                    log_id(3, 0, 6)
                ],
                initial.log_ids.key_log_ids()
            );
        }

        tracing::info!(
            "--- log terms: [x,1,1,2,2,3,3], last_purged_log_id: (0,0), expect [(0,0),(1,1),(2,3),(3,5),(3,6)]"
        );
        {
            store.purge(log_id(0, 0, 0)).await?;

            let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;
            assert_eq!(
                vec![
                    log_id(0, 0, 0),
                    log_id(1, 0, 1),
                    log_id(2, 0, 3),
                    log_id(3, 0, 5),
                    log_id(3, 0, 6)
                ],
                initial.log_ids.key_log_ids()
            );
        }

        tracing::info!("--- log terms: [x,x,1,2,2,3,3], last_purged_log_id: (1,1), expect [(1,1),(2,3),(3,5),(3,6)]");
        {
            store.purge(log_id(1, 0, 1)).await?;

            let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;
            assert_eq!(
                vec![log_id(1, 0, 1), log_id(2, 0, 3), log_id(3, 0, 5), log_id(3, 0, 6)],
                initial.log_ids.key_log_ids()
            );
        }

        tracing::info!("--- log terms: [x,x,x,2,2,3,3], last_purged_log_id: (1,2), expect [(1,2),(2,3),(3,5),(3,6)]");
        {
            store.purge(log_id(1, 0, 2)).await?;

            let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;
            assert_eq!(
                vec![log_id(1, 0, 2), log_id(2, 0, 3), log_id(3, 0, 5), log_id(3, 0, 6)],
                initial.log_ids.key_log_ids()
            );
        }

        tracing::info!("--- log terms: [x,x,x,x,2,3,3], last_purged_log_id: (2,3), expect [(2,3),(3,5),(3,6)]");
        {
            store.purge(log_id(2, 0, 3)).await?;

            let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;
            assert_eq!(
                vec![log_id(2, 0, 3), log_id(3, 0, 5), log_id(3, 0, 6)],
                initial.log_ids.key_log_ids()
            );
        }

        tracing::info!("--- log terms: [x,x,x,x,x,x,x], last_purged_log_id: (3,6), e.g., all purged expect [(3,6)]");
        {
            store.purge(log_id(3, 0, 6)).await?;

            let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;
            assert_eq!(vec![log_id(3, 0, 6)], initial.log_ids.key_log_ids());
        }

        Ok(())
    }

    /// Test if committed logs are re-applied.
    pub async fn get_initial_state_re_apply_committed(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        Self::default_vote(&mut store).await?;

        append(&mut store, [
            blank_ent_0::<C>(1, 2),
            blank_ent_0::<C>(1, 3),
            blank_ent_0::<C>(1, 4),
            blank_ent_0::<C>(1, 5),
        ])
        .await?;
        store.purge(log_id_0(1, 1)).await?;

        apply(&mut sm, [blank_ent_0::<C>(1, 2)]).await?;

        store.save_committed(Some(log_id_0(1, 4))).await?;
        let got = store.read_committed().await?;
        if got.is_none() {
            tracing::info!("This implementation does not store committed log id, skip test re-applying committed logs");
            return Ok(());
        }

        let initial = StorageHelper::new(&mut store, &mut sm).get_initial_state().await?;

        assert_eq!(Some(&log_id_0(1, 4)), initial.io_applied(), "last_applied is updated");
        assert_eq!(
            Some(log_id_0(1, 4)),
            sm.applied_state().await?.0,
            "last_applied is updated"
        );

        Ok(())
    }

    pub async fn save_vote(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        store.save_vote(&Vote::new(100, NODE_ID.into())).await?;

        let got = store.read_vote().await?;

        assert_eq!(Some(Vote::new(100, NODE_ID.into())), got,);
        Ok(())
    }

    pub async fn get_log_entries(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        Self::feed_10_logs_vote_self(&mut store).await?;

        tracing::info!("--- get start == stop");
        {
            let logs = store.try_get_log_entries(3..3).await?;
            assert_eq!(logs.len(), 0, "expected no logs to be returned");
        }

        tracing::info!("--- get start < stop");
        {
            let logs = store.try_get_log_entries(5..7).await?;

            assert_eq!(logs.len(), 2);
            assert_eq!(*logs[0].get_log_id(), log_id_0(1, 5));
            assert_eq!(*logs[1].get_log_id(), log_id_0(1, 6));
        }

        Ok(())
    }

    pub async fn limited_get_log_entries(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        Self::feed_10_logs_vote_self(&mut store).await?;

        tracing::info!("--- get start == stop");
        {
            let logs = store.limited_get_log_entries(3, 3).await?;
            assert_eq!(logs.len(), 0, "expected no logs to be returned");
        }

        tracing::info!("--- get start < stop");
        {
            let logs = store.limited_get_log_entries(5, 7).await?;

            assert!(!logs.is_empty());
            assert!(logs.len() <= 2);
            assert_eq!(*logs[0].get_log_id(), log_id_0(1, 5));
        }

        Ok(())
    }

    pub async fn try_get_log_entry(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        Self::feed_10_logs_vote_self(&mut store).await?;

        store.purge(LogId::new(CommittedLeaderId::new(0, C::NodeId::default()), 0)).await?;

        // `purge()` does not have to do the purge at once.
        // The implementation may choose to do it in the background.
        C::sleep(Duration::from_millis(1_000)).await;

        let ent = store.try_get_log_entry(3).await?;
        assert_eq!(Some(log_id_0(1, 3)), ent.map(|x| *x.get_log_id()));

        let ent = store.try_get_log_entry(0).await?;
        assert_eq!(None, ent.map(|x| *x.get_log_id()));

        let ent = store.try_get_log_entry(11).await?;
        assert_eq!(None, ent.map(|x| *x.get_log_id()));

        Ok(())
    }

    pub async fn initial_logs(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        let ent = store.try_get_log_entry(0).await?;
        assert!(ent.is_none(), "store initialized");

        Ok(())
    }

    pub async fn get_log_state(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        let st = store.get_log_state().await?;

        assert_eq!(None, st.last_purged_log_id);
        assert_eq!(None, st.last_log_id);

        tracing::info!("--- only logs");
        {
            append(&mut store, [
                blank_ent_0::<C>(0, 0),
                blank_ent_0::<C>(1, 1),
                blank_ent_0::<C>(1, 2),
            ])
            .await?;

            let st = store.get_log_state().await?;
            assert_eq!(None, st.last_purged_log_id);
            assert_eq!(Some(log_id_0(1, 2)), st.last_log_id);
        }

        tracing::info!("--- delete log 0-0");
        {
            store.purge(log_id_0(0, 0)).await?;

            let st = store.get_log_state().await?;
            assert_eq!(
                Some(LogId::new(CommittedLeaderId::new(0, C::NodeId::default()), 0)),
                st.last_purged_log_id
            );
            assert_eq!(Some(log_id_0(1, 2)), st.last_log_id);
        }

        tracing::info!("--- delete all log");
        {
            store.purge(log_id_0(1, 2)).await?;

            let st = store.get_log_state().await?;
            assert_eq!(Some(log_id_0(1, 2)), st.last_purged_log_id);
            assert_eq!(Some(log_id_0(1, 2)), st.last_log_id);
        }

        tracing::info!("--- delete advance last present logs");
        {
            store.purge(log_id_0(2, 3)).await?;

            // `purge()` does not have to do the purge at once.
            // The implementation may choose to do it in the background.
            C::sleep(Duration::from_millis(1_000)).await;

            let st = store.get_log_state().await?;
            assert_eq!(Some(log_id_0(2, 3)), st.last_purged_log_id);
            assert_eq!(Some(log_id_0(2, 3)), st.last_log_id);
        }

        Ok(())
    }

    pub async fn get_log_id(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        Self::feed_10_logs_vote_self(&mut store).await?;

        store.purge(log_id_0(1, 3)).await?;

        // `purge()` does not have to do the purge at once.
        // The implementation may choose to do it in the background.
        C::sleep(Duration::from_millis(1_000)).await;

        let res = store.get_log_id(0).await;
        assert!(res.is_err());

        let res = store.get_log_id(3).await;
        assert!(res.is_err());

        let res = store.get_log_id(4).await?;
        assert_eq!(log_id_0(1, 4), res);

        let res = store.get_log_id(10).await?;
        assert_eq!(log_id_0(1, 10), res);

        let res = store.get_log_id(11).await;
        assert!(res.is_err());

        Ok(())
    }

    pub async fn last_id_in_log(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        let last_log_id = store.get_log_state().await?.last_log_id;
        assert_eq!(None, last_log_id);

        tracing::info!("--- only logs");
        {
            append(&mut store, [
                blank_ent_0::<C>(0, 0),
                blank_ent_0::<C>(1, 1),
                blank_ent_0::<C>(1, 2),
            ])
            .await?;

            let last_log_id = store.get_log_state().await?.last_log_id;
            assert_eq!(Some(log_id_0(1, 2)), last_log_id);
        }

        tracing::info!("--- last id in logs < last applied id in sm, only return the id in logs");
        {
            apply(&mut sm, [blank_ent_0::<C>(1, 3)]).await?;
            let last_log_id = store.get_log_state().await?.last_log_id;
            assert_eq!(Some(log_id_0(1, 2)), last_log_id);
        }

        tracing::info!("--- no logs, return default");
        {
            store.purge(log_id_0(1, 2)).await?;

            // `purge()` does not have to do the purge at once.
            // The implementation may choose to do it in the background.
            C::sleep(Duration::from_millis(1_000)).await;

            let last_log_id = store.get_log_state().await?.last_log_id;
            assert_eq!(Some(log_id_0(1, 2)), last_log_id);
        }

        Ok(())
    }

    pub async fn last_applied_state(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        let (applied, mem) = sm.applied_state().await?;
        assert_eq!(None, applied);
        assert_eq!(StoredMembership::default(), mem);

        tracing::info!("--- with last_applied and last_membership");
        {
            apply(&mut sm, [membership_ent_0::<C>(1, 3, btreeset! {1,2})]).await?;

            let (applied, mem) = sm.applied_state().await?;
            assert_eq!(Some(log_id_0(1, 3)), applied);
            assert_eq!(
                StoredMembership::new(Some(log_id_0(1, 3)), Membership::new(vec![btreeset! {1,2}], None)),
                mem
            );
        }

        tracing::info!("--- no logs, return default");
        {
            apply(&mut sm, [blank_ent_0::<C>(1, 5)]).await?;

            let (applied, mem) = sm.applied_state().await?;
            assert_eq!(Some(log_id_0(1, 5)), applied);
            assert_eq!(
                StoredMembership::new(Some(log_id_0(1, 3)), Membership::new(vec![btreeset! {1,2}], None)),
                mem
            );
        }

        Ok(())
    }

    pub async fn purge_logs_upto_0(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        tracing::info!("--- delete (-oo, 0]");

        Self::feed_10_logs_vote_self(&mut store).await?;

        store.purge(log_id_0(0, 0)).await?;

        // `purge()` does not have to do the purge at once.
        // The implementation may choose to do it in the background.
        C::sleep(Duration::from_millis(1_000)).await;

        let logs = store.try_get_log_entries(0..100).await?;
        assert_eq!(logs.len(), 10);
        assert_eq!(logs[0].get_log_id().index, 1);

        assert_eq!(
            LogState {
                last_purged_log_id: Some(log_id_0(0, 0)),
                last_log_id: Some(log_id_0(1, 10)),
            },
            store.get_log_state().await?
        );
        Ok(())
    }

    pub async fn purge_logs_upto_5(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        tracing::info!("--- delete (-oo, 5]");

        Self::feed_10_logs_vote_self(&mut store).await?;

        store.purge(log_id_0(1, 5)).await?;

        // `purge()` does not have to do the purge at once.
        // The implementation may choose to do it in the background.
        C::sleep(Duration::from_millis(1_000)).await;

        let logs = store.try_get_log_entries(0..100).await?;
        assert_eq!(logs.len(), 5);
        assert_eq!(logs[0].get_log_id().index, 6);

        assert_eq!(
            LogState {
                last_purged_log_id: Some(log_id_0(1, 5)),
                last_log_id: Some(log_id_0(1, 10)),
            },
            store.get_log_state().await?
        );
        Ok(())
    }

    pub async fn purge_logs_upto_20(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        tracing::info!("--- delete (-oo, 20]");

        Self::feed_10_logs_vote_self(&mut store).await?;

        store.purge(log_id_0(1, 20)).await?;

        // `purge()` does not have to do the purge at once.
        // The implementation may choose to do it in the background.
        C::sleep(Duration::from_millis(1_000)).await;

        let logs = store.try_get_log_entries(0..100).await?;
        assert_eq!(logs.len(), 0);

        assert_eq!(
            LogState {
                last_purged_log_id: Some(log_id_0(1, 20)),
                last_log_id: Some(log_id_0(1, 20)),
            },
            store.get_log_state().await?
        );
        Ok(())
    }

    pub async fn delete_logs_since_11(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        tracing::info!("--- delete [11, +oo)");

        Self::feed_10_logs_vote_self(&mut store).await?;

        store.truncate(log_id_0(1, 11)).await?;

        let logs = store.try_get_log_entries(0..100).await?;
        assert_eq!(logs.len(), 11);

        assert_eq!(
            LogState {
                last_purged_log_id: None,
                last_log_id: Some(log_id_0(1, 10)),
            },
            store.get_log_state().await?
        );
        Ok(())
    }

    pub async fn delete_logs_since_0(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        tracing::info!("--- delete [0, +oo)");

        Self::feed_10_logs_vote_self(&mut store).await?;

        store.truncate(log_id_0(0, 0)).await?;

        let logs = store.try_get_log_entries(0..100).await?;
        assert_eq!(logs.len(), 0);

        assert_eq!(
            LogState {
                last_purged_log_id: None,
                last_log_id: None,
            },
            store.get_log_state().await?
        );

        Ok(())
    }

    pub async fn append_to_log(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        Self::feed_10_logs_vote_self(&mut store).await?;

        store.purge(log_id_0(0, 0)).await?;

        // `purge()` does not have to do the purge at once.
        // The implementation may choose to do it in the background.
        C::sleep(Duration::from_millis(1_000)).await;

        append(&mut store, [blank_ent_0::<C>(2, 11)]).await?;

        let l = store.try_get_log_entries(0..).await?.len();
        let last = store.try_get_log_entries(0..).await?.into_iter().last().unwrap();

        assert_eq!(l, 11, "expected 11 entries to exist in the log");
        assert_eq!(*last.get_log_id(), log_id_0(2, 11), "unexpected log id");
        Ok(())
    }

    pub async fn snapshot_meta(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        tracing::info!("--- just initialized");
        {
            apply(&mut sm, [membership_ent_0::<C>(0, 0, btreeset! {1,2})]).await?;

            let mut b = sm.get_snapshot_builder().await;
            let snap = b.build_snapshot().await?;
            let meta = snap.meta;
            assert_eq!(Some(log_id_0(0, 0)), meta.last_log_id);
            assert_eq!(&Some(log_id_0(0, 0)), meta.last_membership.log_id());
            assert_eq!(
                &Membership::new(vec![btreeset! {1,2}], None),
                meta.last_membership.membership()
            );
        }

        tracing::info!("--- one app log, one membership log");
        {
            apply(&mut sm, [
                blank_ent_0::<C>(1, 1),
                membership_ent_0::<C>(2, 2, btreeset! {3,4}),
            ])
            .await?;

            let mut b = sm.get_snapshot_builder().await;
            let snap = b.build_snapshot().await?;
            let meta = snap.meta;
            assert_eq!(Some(log_id_0(2, 2)), meta.last_log_id);
            assert_eq!(&Some(log_id_0(2, 2)), meta.last_membership.log_id());
            assert_eq!(
                &Membership::new(vec![btreeset! {3,4}], None),
                meta.last_membership.membership()
            );
        }

        Ok(())
    }

    pub async fn apply_single(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        let (last_applied, _) = sm.applied_state().await?;
        assert_eq!(last_applied, None,);

        tracing::info!("--- apply blank entry");
        {
            let entry = blank_ent_0::<C>(0, 0);

            let replies = apply(&mut sm, [entry]).await?;
            assert_eq!(replies.len(), 1, "expected 1 response");
            let (last_applied, _) = sm.applied_state().await?;

            assert_eq!(last_applied, Some(log_id_0(0, 0)),);
        }

        tracing::info!("--- apply membership entry");
        {
            let entry = membership_ent_0::<C>(1, 1, btreeset! {1,2});

            let replies = apply(&mut sm, [entry]).await?;
            assert_eq!(replies.len(), 1, "expected 1 response");
            let (last_applied, mem) = sm.applied_state().await?;

            assert_eq!(last_applied, Some(log_id_0(1, 1)),);
            assert_eq!(mem.membership(), &Membership::new(vec![btreeset! {1,2}], None));
        }

        // TODO: figure out how to test applying normal entry. `C::D` can not be built by Openraft
        // tracing::info!("--- apply normal entry");
        // {
        //     let entry = {
        //         let mut e = C::Entry::from_app_data(C::D::from(1));
        //         e.set_log_id(&log_id_0(2, 2));
        //         e
        //     };
        //
        //     let replies = apply(&mut sm, [entry]).await?;
        //     assert_eq!(replies.len(), 1, "expected 1 response");
        //     let (last_applied, _) = sm.applied_state().await?;
        //
        //     assert_eq!(last_applied, Some(log_id_0(2, 2)),);
        // }

        Ok(())
    }

    pub async fn apply_multiple(mut store: LS, mut sm: SM) -> Result<(), StorageError<C>> {
        let (last_applied, _) = sm.applied_state().await?;
        assert_eq!(last_applied, None,);

        let entries = [blank_ent_0::<C>(0, 0), membership_ent_0::<C>(1, 1, btreeset! {1,2})];

        let replies = apply(&mut sm, entries).await?;
        assert_eq!(replies.len(), 2);

        let (last_applied, mem) = sm.applied_state().await?;
        assert_eq!(last_applied, Some(log_id_0(1, 1)),);
        assert_eq!(mem.membership(), &Membership::new(vec![btreeset! {1,2}], None));

        Ok(())
    }

    /// Rudimentary test for snapshotting that builds a snapshot on one node and installs it on
    /// another
    pub async fn transfer_snapshot(builder: &B) -> Result<(), StorageError<C>> {
        // Create a snapshot on sm_l, and install it on sm_f
        let (_g_l, _store_l, mut sm_l) = builder.build().await?;
        let (_g_f, _store_f, mut sm_f) = builder.build().await?;

        tracing::info!("--- make sure that initial snapshot is empty");
        // Start with empty snapshot
        assert!(sm_l.get_current_snapshot().await?.is_none(), "initialized snapshot");
        assert!(sm_f.get_current_snapshot().await?.is_none(), "initialized snapshot");

        // Add a few entries so we have state to snapshot
        let snapshot_entries = vec![membership_ent_0::<C>(1, 2, btreeset! {1, 2, 3}), blank_ent_0::<C>(3, 3)];
        sm_l.apply(snapshot_entries).await?;
        let snapshot_last_log_id = Some(log_id_0(3, 3));
        let snapshot_last_membership =
            StoredMembership::new(Some(log_id_0(1, 2)), Membership::new(vec![btreeset![1, 2, 3]], None));
        let snapshot_applied_state = (snapshot_last_log_id, snapshot_last_membership.clone());

        tracing::info!("--- build and get snapshot on leader state machine");
        let ss1 = sm_l.get_snapshot_builder().await.build_snapshot().await?;
        assert_eq!(
            ss1.meta.last_log_id, snapshot_last_log_id,
            "built snapshot has wrong last log id"
        );
        assert_eq!(
            ss1.meta.last_membership, snapshot_last_membership,
            "built snapshot has wrong last membership"
        );
        let ss1_cur = sm_l.get_current_snapshot().await?.expect("uninitialized snapshot");
        assert_eq!(
            ss1_cur.meta, ss1.meta,
            "current snapshot metadata not updated correctly on leader sm"
        );

        tracing::info!("--- install snapshot on follower state machine");
        let mut ss2_box = sm_f.begin_receiving_snapshot().await?;
        *ss2_box = *ss1_cur.snapshot;

        sm_f.install_snapshot(&ss1_cur.meta, ss2_box).await?;

        tracing::info!("--- check correctness of installed snapshot");
        // ... by requesting whole snapshot
        let ss2 = sm_f.get_current_snapshot().await?.expect("uninitialized snapshot");
        assert_eq!(
            ss2.meta, ss1.meta,
            "snapshot metadata not updated correctly on follower sm"
        );
        // ... by checking smstore state
        assert_eq!(sm_f.applied_state().await?, snapshot_applied_state);
        Ok(())
    }

    pub async fn feed_10_logs_vote_self(sto: &mut LS) -> Result<(), StorageError<C>> {
        append(sto, [blank_ent_0::<C>(0, 0)]).await?;

        for i in 1..=10 {
            append(sto, [blank_ent_0::<C>(1, i)]).await?;
        }

        Self::default_vote(sto).await?;

        Ok(())
    }

    pub async fn default_vote(sto: &mut LS) -> Result<(), StorageError<C>> {
        sto.save_vote(&Vote::new(1, NODE_ID.into())).await?;

        Ok(())
    }
}

/// Create a log id with node id 0 for testing.
fn log_id_0<NID>(term: u64, index: u64) -> LogId<NID>
where NID: NodeId + From<u64> {
    LogId {
        leader_id: CommittedLeaderId::new(term, NODE_ID.into()),
        index,
    }
}

/// Create a blank log entry with node_id 0 for test.
fn blank_ent_0<C: RaftTypeConfig>(term: u64, index: u64) -> C::Entry
where C::NodeId: From<u64> {
    C::Entry::new_blank(log_id_0(term, index))
}

/// Create a membership entry with node_id 0 for test.
fn membership_ent_0<C: RaftTypeConfig>(term: u64, index: u64, bs: BTreeSet<C::NodeId>) -> C::Entry
where C::NodeId: From<u64> {
    C::Entry::new_membership(log_id_0(term, index), Membership::new(vec![bs], ()))
}

/// Build a `RaftLogStorage` and `RaftStateMachine` implementation and run a test on it.
async fn run_test<C, LS, SM, G, B, TestFn, Ret, Fu>(builder: &B, test_fn: TestFn) -> Result<Ret, StorageError<C>>
where
    C: RaftTypeConfig,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
    B: StoreBuilder<C, LS, SM, G>,
    Fu: Future<Output = Result<Ret, StorageError<C>>> + OptionalSend,
    TestFn: Fn(LS, SM) -> Fu + Sync + Send,
{
    let (_g, store, sm) = builder.build().await?;
    test_fn(store, sm).await
}

/// A wrapper for calling nonblocking `RaftStateMachine::apply()`
async fn apply<C, SM, I>(sm: &mut SM, entries: I) -> Result<Vec<C::R>, StorageError<C>>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
    I: IntoIterator<Item = C::Entry> + OptionalSend,
    I::IntoIter: OptionalSend,
{
    let resp = sm.apply(entries).await?;
    Ok(resp)
}

/// A wrapper for calling nonblocking `RaftLogStorage::append()`
async fn append<C, LS, I>(store: &mut LS, entries: I) -> Result<(), StorageError<C>>
where
    C: RaftTypeConfig,
    LS: RaftLogStorage<C>,
    I: IntoIterator<Item = C::Entry> + OptionalSend,
    I::IntoIter: OptionalSend,
{
    let entries = entries.into_iter().collect::<Vec<_>>();

    let last_log_id = *entries.last().unwrap().get_log_id();

    let (tx, mut rx) = C::mpsc_unbounded();

    // Dummy log io id for blocking append
    let io_id = IOId::<C>::new_log_io(Vote::<C::NodeId>::default().into_committed(), Some(last_log_id));
    let notify = Notification::LocalIO { io_id };
    let cb = IOFlushed::new(notify, tx.downgrade());

    store.append(entries, cb).await?;
    let got = rx.recv().await.unwrap();
    if let Notification::StorageError { error } = got {
        return Err(error);
    }
    Ok(())
}
