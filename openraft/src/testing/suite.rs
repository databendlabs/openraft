use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::option::Option::None;

use maplit::btreeset;

use crate::membership::EffectiveMembership;
use crate::raft_state::RaftState;
use crate::storage::LogState;
use crate::storage::StorageHelper;
use crate::testing::DefensiveStoreBuilder;
use crate::testing::StoreBuilder;
use crate::AppData;
use crate::AppDataResponse;
use crate::DefensiveError;
use crate::Entry;
use crate::EntryPayload;
use crate::ErrorSubject;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::NodeId;
use crate::RaftSnapshotBuilder;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Violation;
use crate::Vote;

const NODE_ID: u64 = 0;

/// Helper to consturct a `BTreeSet` of `C::NodeId` from numbers.
macro_rules! btreeset {
    ($($key:expr,)+) => (btreeset!($($key),+));
    ( $($key:expr),* ) => {{
        let mut _set = ::std::collections::BTreeSet::new();
        $( _set.insert($key.into()); )*
        _set
    }};
}

/// Test suite to ensure a `RaftStore` impl works as expected.
///
/// Usage:
pub struct Suite<C, S, B>
where
    C: RaftTypeConfig,
    C::D: AppData + Debug,
    C::R: AppDataResponse + Debug,
    S: RaftStorage<C>,
    B: StoreBuilder<C, S>,
{
    c: PhantomData<C>,
    p: PhantomData<S>,
    f: PhantomData<B>,
}

impl<C, S, B> Suite<C, S, B>
where
    C: RaftTypeConfig,
    C::D: AppData + Debug,
    C::R: AppDataResponse + Debug,
    C::NodeId: From<u64>,
    S: RaftStorage<C>,
    B: StoreBuilder<C, S>,
{
    pub fn test_all(builder: B) -> Result<(), StorageError<C::NodeId>> {
        Suite::test_store(&builder)?;

        let df_builder = DefensiveStoreBuilder::<C, S, B> {
            base_builder: builder,

            s: Default::default(),
        };

        Suite::test_store_defensive(&df_builder)?;

        Ok(())
    }

    pub fn test_store(builder: &B) -> Result<(), StorageError<C::NodeId>> {
        run_fut(builder.run_test(Self::last_membership_in_log_initial))?;
        run_fut(builder.run_test(Self::last_membership_in_log))?;
        run_fut(builder.run_test(Self::last_membership_in_log_multi_step))?;
        run_fut(builder.run_test(Self::get_membership_initial))?;
        run_fut(builder.run_test(Self::get_membership_from_log_and_empty_sm))?;
        run_fut(builder.run_test(Self::get_membership_from_log_and_sm))?;
        run_fut(builder.run_test(Self::get_initial_state_without_init))?;
        run_fut(builder.run_test(Self::get_initial_state_membership_from_log_and_sm))?;
        run_fut(builder.run_test(Self::get_initial_state_with_state))?;
        run_fut(builder.run_test(Self::get_initial_state_last_log_gt_sm))?;
        run_fut(builder.run_test(Self::get_initial_state_last_log_lt_sm))?;
        run_fut(builder.run_test(Self::get_initial_state_log_ids))?;
        run_fut(builder.run_test(Self::save_vote))?;
        run_fut(builder.run_test(Self::get_log_entries))?;
        run_fut(builder.run_test(Self::try_get_log_entry))?;
        run_fut(builder.run_test(Self::initial_logs))?;
        run_fut(builder.run_test(Self::get_log_state))?;
        run_fut(builder.run_test(Self::get_log_id))?;
        run_fut(builder.run_test(Self::last_id_in_log))?;
        run_fut(builder.run_test(Self::last_applied_state))?;
        run_fut(builder.run_test(Self::purge_logs_upto_0))?;
        run_fut(builder.run_test(Self::purge_logs_upto_5))?;
        run_fut(builder.run_test(Self::purge_logs_upto_20))?;
        run_fut(builder.run_test(Self::delete_logs_since_11))?;
        run_fut(builder.run_test(Self::delete_logs_since_0))?;
        run_fut(builder.run_test(Self::append_to_log))?;
        run_fut(builder.run_test(Self::snapshot_meta))?;

        // run_fut(Suite::apply_single(builder))?;
        // run_fut(Suite::apply_multi(builder))?;

        // TODO(xp): test: finalized_snapshot, do_log_compaction, begin_receiving_snapshot, get_current_snapshot

        Ok(())
    }

    pub async fn last_membership_in_log_initial(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let membership = StorageHelper::new(&mut store).last_membership_in_log(0).await?;

        assert!(membership.is_empty());

        Ok(())
    }

    pub async fn last_membership_in_log(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("--- no log, do not read membership from state machine");
        {
            store
                .apply_to_state_machine(&[
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 1),
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 2),
                        payload: EntryPayload::Membership(Membership::new(vec![btreeset! {3,4,5}], None)),
                    },
                ])
                .await?;

            let mem = StorageHelper::new(&mut store).last_membership_in_log(0).await?;

            assert!(mem.is_empty());
        }

        tracing::info!("--- membership presents in log, smaller than last_applied, read from log");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 1),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2,3}], None)),
                }])
                .await?;

            let mem = StorageHelper::new(&mut store).last_membership_in_log(0).await?;
            assert_eq!(1, mem.len());
            let mem = mem[0].clone();
            assert_eq!(Membership::new(vec![btreeset! {1, 2, 3}], None), mem.membership,);

            let mem = StorageHelper::new(&mut store).last_membership_in_log(1).await?;
            assert_eq!(1, mem.len());
            let mem = mem[0].clone();
            assert_eq!(Membership::new(vec![btreeset! {1, 2, 3}], None), mem.membership,);

            let mem = StorageHelper::new(&mut store).last_membership_in_log(2).await?;
            assert!(mem.is_empty());
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read 2 membership entries from log");
        {
            store
                .append_to_log(&[
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 2),
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 3),
                        payload: EntryPayload::Membership(Membership::new(vec![btreeset! {7,8,9}], None)),
                    },
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 4),
                        payload: EntryPayload::Blank,
                    },
                ])
                .await?;

            let mems = StorageHelper::new(&mut store).last_membership_in_log(0).await?;
            assert_eq!(2, mems.len());

            let mem = mems[0].clone();
            assert_eq!(Membership::new(vec![btreeset! {1,2,3}], None), mem.membership,);

            let mem = mems[1].clone();
            assert_eq!(Membership::new(vec![btreeset! {7,8,9}], None), mem.membership,);
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read from log but since_index is greater than the last");
        {
            let mem = StorageHelper::new(&mut store).last_membership_in_log(4).await?;
            assert!(mem.is_empty());
        }

        tracing::info!("--- 3 memberships in log, only return the last 2 of them");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 5),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {10,11}], None)),
                }])
                .await?;

            let mems = StorageHelper::new(&mut store).last_membership_in_log(0).await?;
            assert_eq!(2, mems.len());

            let mem = mems[0].clone();
            assert_eq!(Membership::new(vec![btreeset! {7,8,9}], None), mem.membership,);

            let mem = mems[1].clone();
            assert_eq!(Membership::new(vec![btreeset! {10,11}], None), mem.membership,);
        }

        Ok(())
    }

    pub async fn last_membership_in_log_multi_step(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("--- find membership log entry backwards, multiple steps");
        {
            store
                .append_to_log(&[
                    //
                    &Entry {
                        log_id: log_id(1, 1),
                        payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2,3}], None)),
                    },
                    &Entry {
                        log_id: log_id(1, 2),
                        payload: EntryPayload::Membership(Membership::new(vec![btreeset! {3,4,5}], None)),
                    },
                ])
                .await?;

            for i in 3..100 {
                store.append_to_log(&[&blank(1, i)]).await?;
            }

            store
                .append_to_log(&[&Entry {
                    log_id: log_id(1, 100),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {5,6,7}], None)),
                }])
                .await?;

            let mems = StorageHelper::new(&mut store).last_membership_in_log(0).await?;
            assert_eq!(2, mems.len());
            let mem = mems[0].clone();
            assert_eq!(Membership::new(vec![btreeset! {3,4,5}], None), mem.membership,);

            let mem = mems[1].clone();
            assert_eq!(Membership::new(vec![btreeset! {5,6,7}], None), mem.membership,);
        }

        Ok(())
    }

    pub async fn get_membership_initial(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let mem_state = StorageHelper::new(&mut store).get_membership().await?;

        assert_eq!(&EffectiveMembership::default(), mem_state.committed.as_ref());
        assert_eq!(&EffectiveMembership::default(), mem_state.effective.as_ref());

        Ok(())
    }

    pub async fn get_membership_from_log_and_empty_sm(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("--- no log, read membership from state machine");
        {
            // There is an empty membership config in an empty state machine.

            store
                .append_to_log(&[&Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 1),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2,3}], None)),
                }])
                .await?;

            let mem_state = StorageHelper::new(&mut store).get_membership().await?;

            assert_eq!(&EffectiveMembership::default(), mem_state.committed.as_ref());
            assert_eq!(
                Membership::new(vec![btreeset! {1,2,3}], None),
                mem_state.effective.membership,
            );
        }

        Ok(())
    }

    pub async fn get_membership_from_log_and_sm(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("--- no log, read membership from state machine");
        {
            store
                .apply_to_state_machine(&[
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 1),
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 2),
                        payload: EntryPayload::Membership(Membership::new(vec![btreeset! {3,4,5}], None)),
                    },
                ])
                .await?;

            let mem_state = StorageHelper::new(&mut store).get_membership().await?;

            assert_eq!(
                Membership::new(vec![btreeset! {3,4,5}], None),
                mem_state.committed.membership,
            );
            assert_eq!(
                Membership::new(vec![btreeset! {3,4,5}], None),
                mem_state.effective.membership,
            );
        }

        tracing::info!("--- membership presents in log, but smaller than last_applied, read from state machine");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 1),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2,3}], None)),
                }])
                .await?;

            let mem_state = StorageHelper::new(&mut store).get_membership().await?;

            assert_eq!(
                Membership::new(vec![btreeset! {3,4,5}], None),
                mem_state.committed.membership,
            );
            assert_eq!(
                Membership::new(vec![btreeset! {3,4,5}], None),
                mem_state.effective.membership,
            );
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read from log");
        {
            store
                .append_to_log(&[
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 2),
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 3),
                        payload: EntryPayload::Membership(Membership::new(vec![btreeset! {7,8,9}], None)),
                    },
                ])
                .await?;

            let mem_state = StorageHelper::new(&mut store).get_membership().await?;

            assert_eq!(
                Membership::new(vec![btreeset! {3,4,5}], None),
                mem_state.committed.membership,
            );
            assert_eq!(
                Membership::new(vec![btreeset! {7,8,9}], None),
                mem_state.effective.membership,
            );
        }

        tracing::info!("--- two membership present in log and > sm.last_applied, read 2 from log");
        {
            store
                .append_to_log(&[
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 4),
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 5),
                        payload: EntryPayload::Membership(Membership::new(vec![btreeset! {10,11}], None)),
                    },
                ])
                .await?;

            let mem_state = StorageHelper::new(&mut store).get_membership().await?;

            assert_eq!(
                Membership::new(vec![btreeset! {7,8,9}], None),
                mem_state.committed.membership,
            );
            assert_eq!(
                Membership::new(vec![btreeset! {10,11}], None),
                mem_state.effective.membership,
            );
        }

        Ok(())
    }

    pub async fn get_initial_state_without_init(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let initial = StorageHelper::new(&mut store).get_initial_state().await?;
        assert_eq!(RaftState::default(), initial, "uninitialized state");
        Ok(())
    }

    pub async fn get_initial_state_with_state(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        Self::default_vote(&mut store).await?;

        store
            .append_to_log(&[&blank(0, 0), &blank(1, 1), &Entry {
                log_id: LogId::new(LeaderId::new(3, NODE_ID.into()), 2),
                payload: EntryPayload::Blank,
            }])
            .await?;

        store
            .apply_to_state_machine(&[&Entry {
                log_id: LogId::new(LeaderId::new(3, NODE_ID.into()), 1),
                payload: EntryPayload::Blank,
            }])
            .await?;

        let initial = StorageHelper::new(&mut store).get_initial_state().await?;

        assert_eq!(
            initial.last_log_id(),
            Some(LogId::new(LeaderId::new(3, NODE_ID.into()), 2)),
            "state machine has higher log"
        );
        assert_eq!(
            initial.last_applied,
            Some(LogId::new(LeaderId::new(3, NODE_ID.into()), 1)),
            "unexpected value for last applied log"
        );
        assert_eq!(
            Vote {
                term: 1,
                node_id: NODE_ID.into(),
                committed: false,
            },
            initial.vote,
            "unexpected value for default hard state"
        );
        Ok(())
    }

    pub async fn get_initial_state_membership_from_log_and_sm(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        // It should never return membership from logs that are included in state machine present.

        Self::default_vote(&mut store).await?;

        // copy the test from get_membership_config

        tracing::info!("--- no log, read membership from state machine");
        {
            store
                .apply_to_state_machine(&[
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 1),
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 2),
                        payload: EntryPayload::Membership(Membership::new(vec![btreeset! {3,4,5}], None)),
                    },
                ])
                .await?;

            let initial = StorageHelper::new(&mut store).get_initial_state().await?;

            assert_eq!(
                Membership::new(vec![btreeset! {3,4,5}], None),
                initial.membership_state.effective.membership,
            );
        }

        tracing::info!("--- membership presents in log, but smaller than last_applied, read from state machine");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 1),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2,3}], None)),
                }])
                .await?;

            let initial = StorageHelper::new(&mut store).get_initial_state().await?;

            assert_eq!(
                Membership::new(vec![btreeset! {3,4,5}], None),
                initial.membership_state.effective.membership,
            );
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read from log");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 3),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2,3}], None)),
                }])
                .await?;

            let initial = StorageHelper::new(&mut store).get_initial_state().await?;

            assert_eq!(
                Membership::new(vec![btreeset! {1,2,3}], None),
                initial.membership_state.effective.membership,
            );
        }

        Ok(())
    }

    pub async fn get_initial_state_last_log_gt_sm(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        Self::default_vote(&mut store).await?;

        store
            .append_to_log(&[&blank(0, 0), &Entry {
                log_id: LogId::new(LeaderId::new(2, NODE_ID.into()), 1),
                payload: EntryPayload::Blank,
            }])
            .await?;

        store
            .apply_to_state_machine(&[
                &Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 1),
                    payload: EntryPayload::Blank,
                },
                &Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 2),
                    payload: EntryPayload::Blank,
                },
            ])
            .await?;

        let initial = StorageHelper::new(&mut store).get_initial_state().await?;

        assert_eq!(
            initial.last_log_id(),
            Some(LogId::new(LeaderId::new(2, NODE_ID.into()), 1)),
            "state machine has higher log"
        );
        Ok(())
    }

    pub async fn get_initial_state_last_log_lt_sm(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        Self::default_vote(&mut store).await?;

        store.append_to_log(&[&blank(1, 2)]).await?;

        store.apply_to_state_machine(&[&blank(3, 1)]).await?;

        let initial = StorageHelper::new(&mut store).get_initial_state().await?;

        assert_eq!(
            initial.last_log_id(),
            Some(LogId::new(LeaderId::new(3, NODE_ID.into()), 1)),
            "state machine has higher log"
        );
        assert_eq!(
            initial.last_purged_log_id(),
            Some(LogId::new(LeaderId::new(3, NODE_ID.into()), 1)),
            "state machine has higher log"
        );
        Ok(())
    }

    pub async fn get_initial_state_log_ids(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let log_id = |t, n: u64, i| LogId::<C::NodeId> {
            leader_id: LeaderId {
                term: t,
                node_id: n.into(),
            },
            index: i,
        };

        tracing::info!("--- empty store, expect []");
        {
            let initial = StorageHelper::new(&mut store).get_initial_state().await?;
            assert_eq!(Vec::<LogId<C::NodeId>>::new(), initial.log_ids.key_log_ids());
        }

        tracing::info!("--- log terms: [0], last_purged_log_id is None, expect [(0,0)]");
        {
            store.append_to_log(&[&blank(0, 0)]).await?;

            let initial = StorageHelper::new(&mut store).get_initial_state().await?;
            assert_eq!(vec![log_id(0, 0, 0)], initial.log_ids.key_log_ids());
        }

        tracing::info!("--- log terms: [0,1,1,2], last_purged_log_id is None, expect [(0,0),(1,1),(2,3)]");
        {
            store.append_to_log(&[&blank(1, 1), &blank(1, 2), &blank(2, 3)]).await?;

            let initial = StorageHelper::new(&mut store).get_initial_state().await?;
            assert_eq!(
                vec![log_id(0, 0, 0), log_id(1, 0, 1), log_id(2, 0, 3)],
                initial.log_ids.key_log_ids()
            );
        }

        tracing::info!(
            "--- log terms: [0,1,1,2,2,3,3], last_purged_log_id is None, expect [(0,0),(1,1),(2,3),(3,5),(3,6)]"
        );
        {
            store.append_to_log(&[&blank(2, 4), &blank(3, 5), &blank(3, 6)]).await?;

            let initial = StorageHelper::new(&mut store).get_initial_state().await?;
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
            store.purge_logs_upto(log_id(0, 0, 0)).await?;

            let initial = StorageHelper::new(&mut store).get_initial_state().await?;
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
            store.purge_logs_upto(log_id(1, 0, 1)).await?;

            let initial = StorageHelper::new(&mut store).get_initial_state().await?;
            assert_eq!(
                vec![log_id(1, 0, 1), log_id(2, 0, 3), log_id(3, 0, 5), log_id(3, 0, 6)],
                initial.log_ids.key_log_ids()
            );
        }

        tracing::info!("--- log terms: [x,x,x,2,2,3,3], last_purged_log_id: (1,2), expect [(1,2),(2,3),(3,5),(3,6)]");
        {
            store.purge_logs_upto(log_id(1, 0, 2)).await?;

            let initial = StorageHelper::new(&mut store).get_initial_state().await?;
            assert_eq!(
                vec![log_id(1, 0, 2), log_id(2, 0, 3), log_id(3, 0, 5), log_id(3, 0, 6)],
                initial.log_ids.key_log_ids()
            );
        }

        tracing::info!("--- log terms: [x,x,x,x,2,3,3], last_purged_log_id: (2,3), expect [(2,3),(3,5),(3,6)]");
        {
            store.purge_logs_upto(log_id(2, 0, 3)).await?;

            let initial = StorageHelper::new(&mut store).get_initial_state().await?;
            assert_eq!(
                vec![log_id(2, 0, 3), log_id(3, 0, 5), log_id(3, 0, 6)],
                initial.log_ids.key_log_ids()
            );
        }

        tracing::info!("--- log terms: [x,x,x,x,x,x,x], last_purged_log_id: (3,6), e.g., all purged expect [(3,6)]");
        {
            store.purge_logs_upto(log_id(3, 0, 6)).await?;

            let initial = StorageHelper::new(&mut store).get_initial_state().await?;
            assert_eq!(vec![log_id(3, 0, 6)], initial.log_ids.key_log_ids());
        }

        Ok(())
    }

    pub async fn save_vote(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        store
            .save_vote(&Vote {
                term: 100,
                node_id: NODE_ID.into(),
                committed: false,
            })
            .await?;

        let got = store.read_vote().await?;

        assert_eq!(
            Some(Vote {
                term: 100,
                node_id: NODE_ID.into(),
                committed: false,
            }),
            got,
        );
        Ok(())
    }

    pub async fn get_log_entries(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        Self::feed_10_logs_vote_self(&mut store).await?;

        tracing::info!("--- get start == stop");
        {
            let logs = store.get_log_entries(3..3).await?;
            assert_eq!(logs.len(), 0, "expected no logs to be returned");
        }

        tracing::info!("--- get start < stop");
        {
            let logs = store.get_log_entries(5..7).await?;

            assert_eq!(logs.len(), 2);
            assert_eq!(logs[0].log_id, LogId::new(LeaderId::new(1, NODE_ID.into()), 5));
            assert_eq!(logs[1].log_id, LogId::new(LeaderId::new(1, NODE_ID.into()), 6));
        }

        Ok(())
    }

    pub async fn try_get_log_entry(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        Self::feed_10_logs_vote_self(&mut store).await?;

        store.purge_logs_upto(LogId::new(LeaderId::new(0, C::NodeId::default()), 0)).await?;

        let ent = store.try_get_log_entry(3).await?;
        assert_eq!(
            Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 3)),
            ent.map(|x| x.log_id)
        );

        let ent = store.try_get_log_entry(0).await?;
        assert_eq!(None, ent.map(|x| x.log_id));

        let ent = store.try_get_log_entry(11).await?;
        assert_eq!(None, ent.map(|x| x.log_id));

        Ok(())
    }

    pub async fn initial_logs(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let ent = store.try_get_log_entry(0).await?;
        assert!(ent.is_none(), "store initialized");

        Ok(())
    }

    pub async fn get_log_state(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let st = store.get_log_state().await?;

        assert_eq!(None, st.last_purged_log_id);
        assert_eq!(None, st.last_log_id);

        tracing::info!("--- only logs");
        {
            store.append_to_log(&[&blank(0, 0), &blank(1, 1), &blank(1, 2)]).await?;

            let st = store.get_log_state().await?;
            assert_eq!(None, st.last_purged_log_id);
            assert_eq!(Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 2)), st.last_log_id);
        }

        tracing::info!("--- delete log 0-0");
        {
            store.purge_logs_upto(LogId::new(LeaderId::new(0, NODE_ID.into()), 0)).await?;

            let st = store.get_log_state().await?;
            assert_eq!(
                Some(LogId::new(LeaderId::new(0, C::NodeId::default()), 0)),
                st.last_purged_log_id
            );
            assert_eq!(Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 2)), st.last_log_id);
        }

        tracing::info!("--- delete all log");
        {
            store.purge_logs_upto(LogId::new(LeaderId::new(1, NODE_ID.into()), 2)).await?;

            let st = store.get_log_state().await?;
            assert_eq!(
                Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 2)),
                st.last_purged_log_id
            );
            assert_eq!(Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 2)), st.last_log_id);
        }

        tracing::info!("--- delete advance last present logs");
        {
            store.purge_logs_upto(LogId::new(LeaderId::new(2, NODE_ID.into()), 3)).await?;

            let st = store.get_log_state().await?;
            assert_eq!(
                Some(LogId::new(LeaderId::new(2, NODE_ID.into()), 3)),
                st.last_purged_log_id
            );
            assert_eq!(Some(LogId::new(LeaderId::new(2, NODE_ID.into()), 3)), st.last_log_id);
        }

        Ok(())
    }

    pub async fn get_log_id(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        Self::feed_10_logs_vote_self(&mut store).await?;

        store.purge_logs_upto(LogId::new(LeaderId::new(1, NODE_ID.into()), 3)).await?;

        let res = StorageHelper::new(&mut store).get_log_id(0).await;
        assert!(res.is_err());

        let res = StorageHelper::new(&mut store).get_log_id(11).await;
        assert!(res.is_err());

        let res = StorageHelper::new(&mut store).get_log_id(3).await?;
        assert_eq!(LogId::new(LeaderId::new(1, NODE_ID.into()), 3), res);

        let res = StorageHelper::new(&mut store).get_log_id(4).await?;
        assert_eq!(LogId::new(LeaderId::new(1, NODE_ID.into()), 4), res);

        Ok(())
    }

    pub async fn last_id_in_log(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let log_id = store.get_log_state().await?.last_log_id;
        assert_eq!(None, log_id);

        tracing::info!("--- only logs");
        {
            store.append_to_log(&[&blank(0, 0), &blank(1, 1), &blank(1, 2)]).await?;

            let log_id = store.get_log_state().await?.last_log_id;
            assert_eq!(Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 2)), log_id);
        }

        tracing::info!("--- last id in logs < last applied id in sm, only return the id in logs");
        {
            store
                .apply_to_state_machine(&[&Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 3),
                    payload: EntryPayload::Blank,
                }])
                .await?;
            let log_id = store.get_log_state().await?.last_log_id;
            assert_eq!(Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 2)), log_id);
        }

        tracing::info!("--- no logs, return default");
        {
            store.purge_logs_upto(LogId::new(LeaderId::new(1, NODE_ID.into()), 2)).await?;

            let log_id = store.get_log_state().await?.last_log_id;
            assert_eq!(Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 2)), log_id);
        }

        Ok(())
    }

    pub async fn last_applied_state(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let (applied, membership) = store.last_applied_state().await?;
        assert_eq!(None, applied);
        assert_eq!(EffectiveMembership::default(), membership);

        tracing::info!("--- with last_applied and last_membership");
        {
            store
                .apply_to_state_machine(&[&Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 3),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2}], None)),
                }])
                .await?;

            let (applied, membership) = store.last_applied_state().await?;
            assert_eq!(Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 3)), applied);
            assert_eq!(
                EffectiveMembership::new(
                    Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 3)),
                    Membership::new(vec![btreeset! {1,2}], None)
                ),
                membership
            );
        }

        tracing::info!("--- no logs, return default");
        {
            store
                .apply_to_state_machine(&[&Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 5),
                    payload: EntryPayload::Blank,
                }])
                .await?;

            let (applied, membership) = store.last_applied_state().await?;
            assert_eq!(Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 5)), applied);
            assert_eq!(
                EffectiveMembership::new(
                    Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 3)),
                    Membership::new(vec![btreeset! {1,2}], None)
                ),
                membership
            );
        }

        Ok(())
    }

    pub async fn purge_logs_upto_0(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("--- delete (-oo, 0]");

        Self::feed_10_logs_vote_self(&mut store).await?;

        store.purge_logs_upto(LogId::new(LeaderId::new(0, NODE_ID.into()), 0)).await?;

        let logs = store.try_get_log_entries(0..100).await?;
        assert_eq!(logs.len(), 10);
        assert_eq!(logs[0].log_id.index, 1);

        assert_eq!(
            LogState {
                last_purged_log_id: Some(LogId::new(LeaderId::new(0, NODE_ID.into()), 0)),
                last_log_id: Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 10)),
            },
            store.get_log_state().await?
        );
        Ok(())
    }

    pub async fn purge_logs_upto_5(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("--- delete (-oo, 5]");

        Self::feed_10_logs_vote_self(&mut store).await?;

        store.purge_logs_upto(LogId::new(LeaderId::new(1, NODE_ID.into()), 5)).await?;

        let logs = store.try_get_log_entries(0..100).await?;
        assert_eq!(logs.len(), 5);
        assert_eq!(logs[0].log_id.index, 6);

        assert_eq!(
            LogState {
                last_purged_log_id: Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 5)),
                last_log_id: Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 10)),
            },
            store.get_log_state().await?
        );
        Ok(())
    }

    pub async fn purge_logs_upto_20(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("--- delete (-oo, 20]");

        Self::feed_10_logs_vote_self(&mut store).await?;

        store.purge_logs_upto(LogId::new(LeaderId::new(1, NODE_ID.into()), 20)).await?;

        let logs = store.try_get_log_entries(0..100).await?;
        assert_eq!(logs.len(), 0);

        assert_eq!(
            LogState {
                last_purged_log_id: Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 20)),
                last_log_id: Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 20)),
            },
            store.get_log_state().await?
        );
        Ok(())
    }

    pub async fn delete_logs_since_11(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("--- delete [11, +oo)");

        Self::feed_10_logs_vote_self(&mut store).await?;

        store.delete_conflict_logs_since(LogId::new(LeaderId::new(1, NODE_ID.into()), 11)).await?;

        let logs = store.try_get_log_entries(0..100).await?;
        assert_eq!(logs.len(), 11);

        assert_eq!(
            LogState {
                last_purged_log_id: None,
                last_log_id: Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 10)),
            },
            store.get_log_state().await?
        );
        Ok(())
    }

    pub async fn delete_logs_since_0(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("--- delete [0, +oo)");

        Self::feed_10_logs_vote_self(&mut store).await?;

        store.delete_conflict_logs_since(LogId::new(LeaderId::new(0, NODE_ID.into()), 0)).await?;

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

    pub async fn append_to_log(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        Self::feed_10_logs_vote_self(&mut store).await?;

        store.purge_logs_upto(LogId::new(LeaderId::new(0, NODE_ID.into()), 0)).await?;

        store.append_to_log(&[&blank(2, 10)]).await?;

        let l = store.try_get_log_entries(0..).await?.len();
        let last = store.try_get_log_entries(0..).await?.last().cloned().unwrap();

        assert_eq!(l, 10, "expected 10 entries to exist in the log");
        assert_eq!(
            last.log_id,
            LogId::new(LeaderId::new(2, NODE_ID.into()), 10),
            "unexpected log id"
        );
        Ok(())
    }

    pub async fn snapshot_meta(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("--- just initialized");
        {
            store
                .apply_to_state_machine(&[
                    //
                    &Entry {
                        log_id: log_id(0, 0),
                        payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2}], None)),
                    },
                ])
                .await?;

            let mut b = store.get_snapshot_builder().await;
            let snap = b.build_snapshot().await?;
            let meta = snap.meta;
            assert_eq!(log_id(0, 0), meta.last_log_id);
            assert_eq!(Some(log_id(0, 0)), meta.last_membership.log_id);
            assert_eq!(
                Membership::new(vec![btreeset! {1,2}], None),
                meta.last_membership.membership
            );
        }

        tracing::info!("--- one app log, one membership log");
        {
            store
                .apply_to_state_machine(&[
                    //
                    &blank(1, 1),
                    &Entry {
                        log_id: log_id(2, 2),
                        payload: EntryPayload::Membership(Membership::new(vec![btreeset! {3,4}], None)),
                    },
                ])
                .await?;

            let mut b = store.get_snapshot_builder().await;
            let snap = b.build_snapshot().await?;
            let meta = snap.meta;
            assert_eq!(log_id(2, 2), meta.last_log_id);
            assert_eq!(Some(log_id(2, 2)), meta.last_membership.log_id);
            assert_eq!(
                Membership::new(vec![btreeset! {3,4}], None),
                meta.last_membership.membership
            );
        }

        Ok(())
    }

    // pub async fn apply_single(mut store: S) -> Result<(), StorageError<C::NodeId>> {

    //
    //     let entry = Entry {
    //         log_id: LogId::new(LeaderId::new(3, NODE_ID.into()), 1),
    //
    //         payload: EntryPayload::Normal(ClientRequest {
    //             client: "0".into(),
    //             serial: 0,
    //             status: "lit".into(),
    //         }),
    //     };
    //
    //     store.apply_to_state_machine(&[&entry]).await?;
    //     let (last_applied, _) = store.last_applied_state().await?;
    //
    //     assert_eq!(
    //         last_applied,
    //         Some(LogId::new(LeaderId::new(3, NODE_ID.into()), 1)),
    //         "expected last_applied_log to be 1, got {:?}",
    //         last_applied
    //     );
    //
    //     let sm = store.get_state_machine().await;
    //     let client_serial =
    //         sm.client_serial_responses.get("0").expect("expected entry to exist in client_serial_responses");
    //     assert_eq!(client_serial, &(0, None), "unexpected client serial response");
    //
    //     let client_status = sm.client_status.get("0").expect("expected entry to exist in client_status");
    //     assert_eq!(
    //         client_status, "lit",
    //         "expected client_status to be 'lit', got '{}'",
    //         client_status
    //     );
    //     Ok(())
    // }
    //
    // pub async fn apply_multi(mut store: S) -> Result<(), StorageError<C::NodeId>> {

    //
    //     let req0 = ClientRequest {
    //         client: "1".into(),
    //         serial: 0,
    //         status: "old".into(),
    //     };
    //     let req1 = ClientRequest {
    //         client: "1".into(),
    //         serial: 1,
    //         status: "new".into(),
    //     };
    //     let req2 = ClientRequest {
    //         client: "2".into(),
    //         serial: 0,
    //         status: "other".into(),
    //     };
    //
    //     let entries = vec![
    //         (&LogId::new(LeaderId::new(3, NODE_ID.into()), 1), &req0),
    //         (&LogId::new(LeaderId::new(3, NODE_ID.into()), 2), &req1),
    //         (&LogId::new(LeaderId::new(3, NODE_ID.into()), 3), &req2),
    //     ]
    //     .into_iter()
    //     .map(|(id, req)| Entry {
    //         log_id: *id,
    //         payload: EntryPayload::Normal(req.clone()),
    //     })
    //     .collect::<Vec<_>>();
    //
    //     store.apply_to_state_machine(&entries.iter().collect::<Vec<_>>()).await?;
    //
    //     let (last_applied, _) = store.last_applied_state().await?;
    //
    //     assert_eq!(
    //         last_applied,
    //         Some(LogId::new(LeaderId::new(3, NODE_ID.into()), 3)),
    //         "expected last_applied_log to be 3, got {:?}",
    //         last_applied
    //     );
    //
    //     let sm = store.get_state_machine().await;
    //
    //     let client_serial1 = sm
    //         .client_serial_responses
    //         .get("1")
    //         .expect("expected entry to exist in client_serial_responses for client 1");
    //     assert_eq!(client_serial1.0, 1, "unexpected client serial response");
    //     assert_eq!(
    //         client_serial1.1,
    //         Some(String::from("old")),
    //         "unexpected client serial response"
    //     );
    //
    //     let client_serial2 = sm
    //         .client_serial_responses
    //         .get("2")
    //         .expect("expected entry to exist in client_serial_responses for client 2");
    //     assert_eq!(client_serial2.0, 0, "unexpected client serial response");
    //     assert_eq!(client_serial2.1, None, "unexpected client serial response");
    //
    //     let client_status1 = sm.client_status.get("1").expect("expected entry to exist in client_status for client
    // 1");     let client_status2 = sm.client_status.get("2").expect("expected entry to exist in client_status for
    // client 2");     assert_eq!(
    //         client_status1, "new",
    //         "expected client_status to be 'new', got '{}'",
    //         client_status1
    //     );
    //     assert_eq!(
    //         client_status2, "other",
    //         "expected client_status to be 'other', got '{}'",
    //         client_status2
    //     );
    //     Ok(())
    // }

    pub async fn feed_10_logs_vote_self(sto: &mut S) -> Result<(), StorageError<C::NodeId>> {
        sto.append_to_log(&[&blank(0, 0)]).await?;

        for i in 1..=10 {
            sto.append_to_log(&[&Entry {
                log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), i),
                payload: EntryPayload::Blank,
            }])
            .await?;
        }

        Self::default_vote(sto).await?;

        Ok(())
    }

    pub async fn default_vote(sto: &mut S) -> Result<(), StorageError<C::NodeId>> {
        sto.save_vote(&Vote {
            term: 1,
            node_id: NODE_ID.into(),
            committed: false,
        })
        .await?;

        Ok(())
    }
}

// Defensive test:
// If a RaftStore impl support defensive check, enable it and check if it returns errors when abnormal input is seen.
// A RaftStore with defensive check is able to expose bugs in raft core.
impl<C, S, B> Suite<C, S, B>
where
    C: RaftTypeConfig,
    C::D: AppData + Debug,
    C::R: AppDataResponse + Debug,
    C::NodeId: From<u64>,
    S: RaftStorage<C>,
    B: StoreBuilder<C, S>,
{
    pub fn test_store_defensive(builder: &B) -> Result<(), StorageError<C::NodeId>> {
        run_fut(builder.run_test(Self::df_get_membership_config_dirty_log))?;
        run_fut(builder.run_test(Self::df_get_initial_state_dirty_log))?;
        run_fut(builder.run_test(Self::df_save_vote_ascending))?;
        run_fut(builder.run_test(Self::df_get_log_entries))?;
        run_fut(builder.run_test(Self::df_append_to_log_nonempty_input))?;
        run_fut(builder.run_test(Self::df_append_to_log_nonconsecutive_input))?;
        run_fut(builder.run_test(Self::df_append_to_log_eq_last_plus_one))?;
        run_fut(builder.run_test(Self::df_append_to_log_eq_last_applied_plus_one))?;
        run_fut(builder.run_test(Self::df_append_to_log_gt_last_log_id))?;
        run_fut(builder.run_test(Self::df_append_to_log_gt_last_applied_id))?;
        run_fut(builder.run_test(Self::df_apply_nonempty_input))?;
        run_fut(builder.run_test(Self::df_apply_index_eq_last_applied_plus_one))?;
        run_fut(builder.run_test(Self::df_apply_gt_last_applied_id))?;
        run_fut(builder.run_test(Self::df_purge_applied_le_last_applied))?;
        run_fut(builder.run_test(Self::df_delete_conflict_gt_last_applied))?;

        Ok(())
    }

    pub async fn df_get_membership_config_dirty_log(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("--- dirty log: log.index > last_applied.index && log < last_applied");
        {
            store
                .append_to_log(&[&blank(0, 0), &blank(1, 1), &blank(1, 2), &Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 3),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2,3}], None)),
                }])
                .await?;
            store
                .apply_to_state_machine(&[&blank(0, 0), &blank(2, 1), &Entry {
                    log_id: LogId::new(LeaderId::new(2, NODE_ID.into()), 2),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {3,4,5}], None)),
                }])
                .await?;

            let res = StorageHelper::new(&mut store).get_membership().await;

            let e = res.unwrap_err().into_defensive().unwrap();
            assert!(matches!(e, DefensiveError {
                subject: ErrorSubject::Log(LogId {
                    leader_id: LeaderId {
                        term: 1,
                        node_id,
                    },
                    index: 3
                },),
                violation: Violation::DirtyLog {
                    higher_index_log_id: LogId {
                        leader_id: LeaderId {
                            term: 1,
                            node_id: node_id2,
                        },
                        index: 3
                    },
                    lower_index_log_id: LogId {
                        leader_id: LeaderId {
                            term: 2,
                            node_id: node_id3,
                        },
                        index: 2
                    },
                },
                ..
            } if node_id == NODE_ID.into() && node_id2 == NODE_ID.into() && node_id3 == NODE_ID.into()))
        }

        Ok(())
    }

    pub async fn df_get_initial_state_dirty_log(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("--- dirty log: log.index > last_applied.index && log < last_applied");
        {
            store
                .append_to_log(&[&blank(0, 0), &blank(1, 1), &blank(1, 2), &Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 3),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2,3}], None)),
                }])
                .await?;

            store
                .apply_to_state_machine(&[&blank(0, 0), &blank(2, 1), &Entry {
                    log_id: LogId::new(LeaderId::new(2, NODE_ID.into()), 2),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {3,4,5}], None)),
                }])
                .await?;

            let state = StorageHelper::new(&mut store).get_initial_state().await;
            let e = state.unwrap_err().into_defensive().unwrap();

            assert!(matches!(e, DefensiveError {
                subject: ErrorSubject::Log(LogId {
                    leader_id: LeaderId {
                        term: 1,
                        node_id,
                    },
                    index: 3
                }),
                violation: Violation::DirtyLog {
                    higher_index_log_id: LogId {
                        leader_id: LeaderId {
                            term: 1,
                            node_id: node_id2,
                        },
                        index: 3
                    },
                    lower_index_log_id: LogId {
                        leader_id: LeaderId {
                            term: 2,
                            node_id: node_id3,
                        },
                        index: 2
                    },
                },
                ..
            } if node_id ==  NODE_ID.into() && node_id2 ==  NODE_ID.into() && node_id3 ==  NODE_ID.into()))
        }

        Ok(())
    }

    pub async fn df_save_vote_ascending(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        store
            .save_vote(&Vote {
                term: 10,
                node_id: 10.into(),
                committed: false,
            })
            .await?;

        tracing::info!("--- lower term is rejected");
        {
            let res = store
                .save_vote(&Vote {
                    term: 9,
                    node_id: 10.into(),
                    committed: false,
                })
                .await;

            let e = res.unwrap_err().into_defensive().unwrap();
            assert!(matches!(e, DefensiveError {
                subject: ErrorSubject::Vote,
                violation: Violation::TermNotAscending { curr: 10, to: 9 },
                ..
            }));

            let vote = store.read_vote().await?;

            assert_eq!(
                Some(Vote {
                    term: 10,
                    node_id: 10.into(),
                    committed: false
                }),
                vote,
            );
        }

        tracing::info!("--- lower node_id is rejected");
        {
            let res = store
                .save_vote(&Vote {
                    term: 10,
                    node_id: 9.into(),
                    committed: false,
                })
                .await;

            let e = res.unwrap_err().into_defensive().unwrap();
            assert!(matches!(e, DefensiveError {
                subject: ErrorSubject::Vote,
                violation: Violation::NonIncrementalVote {
                    curr: Vote {
                        term: 10,
                        node_id,
                        committed: false,
                    },
                    to: Vote {
                        term: 10,
                        node_id: node_id2,
                        committed: false,
                    }
                },
                ..
            } if node_id == 10.into() && node_id2 == 9.into()));

            let vote = store.read_vote().await?;

            assert_eq!(
                Some(Vote {
                    term: 10,
                    node_id: 10.into(),
                    committed: false
                }),
                vote
            );
        }

        Ok(())
    }

    pub async fn df_get_log_entries(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        Self::feed_10_logs_vote_self(&mut store).await?;

        store.apply_to_state_machine(&[&blank(0, 0)]).await?;

        store.purge_logs_upto(LogId::new(LeaderId::new(0, C::NodeId::default()), 0)).await?;

        store.get_log_entries(..).await?;
        store.get_log_entries(5..).await?;
        store.get_log_entries(..5).await?;
        store.get_log_entries(5..7).await?;

        // mismatched bound.

        let res = store.get_log_entries(11..).await;
        let e = res.unwrap_err().into_defensive().unwrap();
        assert!(matches!(e, DefensiveError {
            subject: ErrorSubject::LogIndex(11),
            violation: Violation::LogIndexNotFound { want: 11, got: None },
            ..
        }));

        let res = store.get_log_entries(1..1).await;
        let e = res.unwrap_err().into_defensive().unwrap();
        assert!(matches!(e, DefensiveError {
            subject: ErrorSubject::Logs,
            violation: Violation::RangeEmpty {
                start: Some(1),
                end: Some(0)
            },
            ..
        }));

        let res = store.get_log_entries(0..1).await;
        let e = res.unwrap_err().into_defensive().unwrap();
        assert!(matches!(e, DefensiveError {
            subject: ErrorSubject::LogIndex(0),
            violation: Violation::LogIndexNotFound { want: 0, got: None },
            ..
        }));

        let res = store.get_log_entries(0..2).await;
        let e = res.unwrap_err().into_defensive().unwrap();
        assert!(matches!(e, DefensiveError {
            subject: ErrorSubject::LogIndex(0),
            violation: Violation::LogIndexNotFound { want: 0, got: Some(1) },
            ..
        }));

        let res = store.get_log_entries(10..12).await;
        let e = res.unwrap_err().into_defensive().unwrap();
        assert!(matches!(e, DefensiveError {
            subject: ErrorSubject::LogIndex(11),
            violation: Violation::LogIndexNotFound {
                want: 11,
                got: Some(10)
            },
            ..
        }));

        Ok(())
    }

    pub async fn df_append_to_log_nonempty_input(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let res = store.append_to_log(Vec::<&Entry<_>>::new().as_slice()).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(ErrorSubject::Logs, e.subject);
        assert_eq!(Violation::LogsEmpty, e.violation);

        Ok(())
    }

    pub async fn df_append_to_log_nonconsecutive_input(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let res = store
            .append_to_log(&[
                &Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 1),
                    payload: EntryPayload::Blank,
                },
                &Entry {
                    log_id: LogId::new(LeaderId::new(1, NODE_ID.into()), 3),
                    payload: EntryPayload::Blank,
                },
            ])
            .await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(ErrorSubject::Logs, e.subject);
        assert_eq!(
            Violation::LogsNonConsecutive {
                prev: Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 1)),
                next: LogId::new(LeaderId::new(1, NODE_ID.into()), 3),
            },
            e.violation
        );

        Ok(())
    }

    pub async fn df_append_to_log_eq_last_plus_one(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("-- log_id <= last_applied");
        tracing::info!("-- nonconsecutive log");
        tracing::info!("-- overlapping log");

        store.append_to_log(&[&blank(0, 0), &blank(1, 1), &blank(1, 2)]).await?;

        store.apply_to_state_machine(&[&blank(0, 0), &blank(1, 1)]).await?;

        let res = store.append_to_log(&[&blank(3, 4)]).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(
            ErrorSubject::Log(LogId::new(LeaderId::new(3, NODE_ID.into()), 4)),
            e.subject
        );
        assert_eq!(
            Violation::LogsNonConsecutive {
                prev: Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 2)),
                next: LogId::new(LeaderId::new(3, NODE_ID.into()), 4),
            },
            e.violation
        );

        Ok(())
    }

    pub async fn df_append_to_log_eq_last_applied_plus_one(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        // last_log: 1,1
        // last_applied: 1,2
        // append_to_log: 1,4

        tracing::info!("-- log_id <= last_applied");
        tracing::info!("-- nonconsecutive log");
        tracing::info!("-- overlapping log");

        store.append_to_log(&[&blank(0, 0), &blank(1, 1), &blank(1, 2)]).await?;

        store.apply_to_state_machine(&[&blank(0, 0), &blank(1, 1), &blank(1, 2)]).await?;

        let res = store.append_to_log(&[&blank(1, 4)]).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(
            ErrorSubject::Log(LogId::new(LeaderId::new(1, NODE_ID.into()), 4)),
            e.subject
        );
        assert_eq!(
            Violation::LogsNonConsecutive {
                prev: Some(LogId::new(LeaderId::new(1, NODE_ID.into()), 2)),
                next: LogId::new(LeaderId::new(1, NODE_ID.into()), 4),
            },
            e.violation
        );

        Ok(())
    }

    pub async fn df_append_to_log_gt_last_log_id(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        // last_log: 2,2
        // append_to_log: 1,3: index == last + 1 but term is lower

        store.append_to_log(&[&blank(0, 0), &blank(2, 1), &blank(2, 2)]).await?;

        let res = store.append_to_log(&[&blank(1, 3)]).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(
            ErrorSubject::Log(LogId::new(LeaderId::new(1, NODE_ID.into()), 3)),
            e.subject
        );
        assert_eq!(
            Violation::LogsNonConsecutive {
                prev: Some(LogId::new(LeaderId::new(2, NODE_ID.into()), 2)),
                next: LogId::new(LeaderId::new(1, NODE_ID.into()), 3),
            },
            e.violation
        );

        Ok(())
    }

    pub async fn df_append_to_log_gt_last_applied_id(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        // last_log: 2,1
        // last_applied: 2,2
        // append_to_log: 1,3: index == last + 1 but term is lower

        store.append_to_log(&[&blank(0, 0), &blank(2, 1), &blank(2, 2)]).await?;

        store.apply_to_state_machine(&[&blank(0, 0), &blank(2, 1), &blank(2, 2)]).await?;

        store.purge_logs_upto(LogId::new(LeaderId::new(2, NODE_ID.into()), 2)).await?;

        let res = store.append_to_log(&[&blank(1, 3)]).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(
            ErrorSubject::Log(LogId::new(LeaderId::new(1, NODE_ID.into()), 3)),
            e.subject
        );
        assert_eq!(
            Violation::LogsNonConsecutive {
                prev: Some(LogId::new(LeaderId::new(2, NODE_ID.into()), 2)),
                next: LogId::new(LeaderId::new(1, NODE_ID.into()), 3),
            },
            e.violation
        );

        Ok(())
    }

    pub async fn df_apply_nonempty_input(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let res = store.apply_to_state_machine(Vec::<&Entry<_>>::new().as_slice()).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(ErrorSubject::Logs, e.subject);
        assert_eq!(Violation::LogsEmpty, e.violation);

        Ok(())
    }

    pub async fn df_apply_index_eq_last_applied_plus_one(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let entry = blank(3, 1);

        store.apply_to_state_machine(&[&blank(0, 0), &entry]).await?;

        tracing::info!("--- re-apply 1th");
        {
            let res = store.apply_to_state_machine(&[&entry]).await;

            let e = res.unwrap_err().into_defensive().unwrap();
            assert_eq!(
                ErrorSubject::Apply(LogId::new(LeaderId::new(3, NODE_ID.into()), 1)),
                e.subject
            );
            assert_eq!(
                Violation::ApplyNonConsecutive {
                    prev: Some(LogId::new(LeaderId::new(3, NODE_ID.into()), 1)),
                    next: LogId::new(LeaderId::new(3, NODE_ID.into()), 1),
                },
                e.violation
            );
        }

        tracing::info!("--- apply 3rd when there is only 1st");
        {
            let entry = blank(3, 3);
            let res = store.apply_to_state_machine(&[&entry]).await;

            let e = res.unwrap_err().into_defensive().unwrap();
            assert_eq!(
                ErrorSubject::Apply(LogId::new(LeaderId::new(3, NODE_ID.into()), 3)),
                e.subject
            );
            assert_eq!(
                Violation::ApplyNonConsecutive {
                    prev: Some(LogId::new(LeaderId::new(3, NODE_ID.into()), 1)),
                    next: LogId::new(LeaderId::new(3, NODE_ID.into()), 3),
                },
                e.violation
            );
        }

        Ok(())
    }

    pub async fn df_apply_gt_last_applied_id(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        let entry = blank(3, 1);

        store.apply_to_state_machine(&[&blank(0, 0), &entry]).await?;

        tracing::info!("--- next apply with last_index+1 but lower term");
        {
            let entry = blank(2, 2);
            let res = store.apply_to_state_machine(&[&entry]).await;
            assert!(res.is_err());

            let e = res.unwrap_err().into_defensive().unwrap();
            assert_eq!(
                ErrorSubject::Apply(LogId::new(LeaderId::new(2, NODE_ID.into()), 2)),
                e.subject
            );
            assert_eq!(
                Violation::ApplyNonConsecutive {
                    prev: Some(LogId::new(LeaderId::new(3, NODE_ID.into()), 1)),
                    next: LogId::new(LeaderId::new(2, NODE_ID.into()), 2),
                },
                e.violation
            );
        }

        Ok(())
    }

    pub async fn df_purge_applied_le_last_applied(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        store.apply_to_state_machine(&[&blank(0, 0), &blank(3, 1)]).await?;

        {
            let res = store.purge_logs_upto(LogId::new(LeaderId::new(5, NODE_ID.into()), 2)).await;
            assert!(res.is_err());

            let e = res.unwrap_err().into_defensive().unwrap();
            assert_eq!(
                ErrorSubject::Log(LogId::new(LeaderId::new(5, NODE_ID.into()), 2)),
                e.subject
            );
            assert_eq!(
                Violation::PurgeNonApplied {
                    last_applied: Some(LogId::new(LeaderId::new(3, NODE_ID.into()), 1)),
                    purge_upto: LogId::new(LeaderId::new(5, NODE_ID.into()), 2),
                },
                e.violation
            );
        }

        Ok(())
    }

    pub async fn df_delete_conflict_gt_last_applied(mut store: S) -> Result<(), StorageError<C::NodeId>> {
        store.apply_to_state_machine(&[&blank(0, 0), &blank(3, 1)]).await?;

        {
            let res = store.delete_conflict_logs_since(LogId::new(LeaderId::new(5, NODE_ID.into()), 1)).await;
            assert!(res.is_err());

            let e = res.unwrap_err().into_defensive().unwrap();
            assert_eq!(
                ErrorSubject::Log(LogId::new(LeaderId::new(5, NODE_ID.into()), 1)),
                e.subject
            );
            assert_eq!(
                Violation::AppliedWontConflict {
                    last_applied: Some(LogId::new(LeaderId::new(3, NODE_ID.into()), 1)),
                    first_conflict_log_id: LogId::new(LeaderId::new(5, NODE_ID.into()), 1),
                },
                e.violation
            );
        }

        Ok(())
    }
}

fn log_id<NID: NodeId>(term: u64, index: u64) -> LogId<NID>
where NID: From<u64> {
    LogId {
        leader_id: LeaderId {
            term,
            node_id: NODE_ID.into(),
        },
        index,
    }
}

/// Create a blank log entry for test
fn blank<C: RaftTypeConfig>(term: u64, index: u64) -> Entry<C>
where C::NodeId: From<u64> {
    Entry {
        log_id: LogId::new(LeaderId::new(term, NODE_ID.into()), index),
        payload: EntryPayload::Blank,
    }
}

/// Block until a future is finished.
/// The future will be running in a clean tokio runtime, to prevent an unfinished task affecting the test.
pub fn run_fut<NID: NodeId, F>(f: F) -> Result<(), StorageError<NID>>
where F: Future<Output = Result<(), StorageError<NID>>> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(f)?;
    Ok(())
}
