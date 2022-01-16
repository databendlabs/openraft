use std::collections::Bound;
use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;

use maplit::btreeset;

use crate::raft::Entry;
use crate::raft::EntryPayload;
use crate::storage::HardState;
use crate::storage::InitialState;
use crate::testing::DefensiveStoreBuilder;
use crate::testing::StoreBuilder;
use crate::AppData;
use crate::AppDataResponse;
use crate::DefensiveError;
use crate::EffectiveMembership;
use crate::ErrorSubject;
use crate::LogId;
use crate::Membership;
use crate::RaftStorage;
use crate::Violation;

const NODE_ID: u64 = 0;

/// Test suite to ensure a `RaftStore` impl works as expected.
///
/// Usage:
pub struct Suite<D, R, S, B>
where
    D: AppData + Debug,
    R: AppDataResponse + Debug,
    S: RaftStorage<D, R>,
    B: StoreBuilder<D, R, S>,
{
    d: PhantomData<D>,
    r: PhantomData<R>,
    p: PhantomData<S>,
    f: PhantomData<B>,
}

impl<D, R, S, B> Suite<D, R, S, B>
where
    D: AppData + Debug,
    R: AppDataResponse + Debug,
    S: RaftStorage<D, R>,
    B: StoreBuilder<D, R, S>,
{
    pub fn test_all(builder: B) -> anyhow::Result<()> {
        Suite::test_store(&builder)?;

        let df_builder = DefensiveStoreBuilder::<D, R, S, B> {
            base_builder: builder,

            d: Default::default(),
            r: Default::default(),
            s: Default::default(),
        };

        Suite::test_store_defensive(&df_builder)?;

        Ok(())
    }

    pub fn test_store(builder: &B) -> anyhow::Result<()> {
        println!("---");
        run_fut(Suite::last_membership_in_log_initial(builder))?;
        run_fut(Suite::last_membership_in_log(builder))?;
        run_fut(Suite::get_membership_initial(builder))?;
        run_fut(Suite::get_membership_from_log_and_sm(builder))?;
        run_fut(Suite::get_initial_state_without_init(builder))?;
        run_fut(Suite::get_initial_state_membership_from_log_and_sm(builder))?;
        run_fut(Suite::get_initial_state_with_state(builder))?;
        run_fut(Suite::get_initial_state_last_log_gt_sm(builder))?;
        run_fut(Suite::get_initial_state_last_log_lt_sm(builder))?;
        run_fut(Suite::save_hard_state(builder))?;
        run_fut(Suite::get_log_entries(builder))?;
        run_fut(Suite::try_get_log_entry(builder))?;
        run_fut(Suite::initial_logs(builder))?;
        run_fut(Suite::first_known_log_id(builder))?;
        run_fut(Suite::get_log_state(builder))?;
        run_fut(Suite::first_id_in_log(builder))?;
        run_fut(Suite::last_id_in_log(builder))?;
        run_fut(Suite::last_applied_state(builder))?;
        run_fut(Suite::delete_logs_from(builder))?;
        run_fut(Suite::append_to_log(builder))?;
        // run_fut(Suite::apply_single(builder))?;
        // run_fut(Suite::apply_multi(builder))?;

        // TODO(xp): test: finalized_snapshot, do_log_compaction, begin_receiving_snapshot, get_current_snapshot

        Ok(())
    }

    pub async fn last_membership_in_log_initial(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let membership = store.last_membership_in_log(0).await?;

        assert!(membership.is_none());

        Ok(())
    }

    pub async fn last_membership_in_log(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        tracing::info!("--- no log, do not read membership from state machine");
        {
            store
                .apply_to_state_machine(&[
                    &Entry {
                        log_id: LogId { term: 1, index: 1 },
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 1, index: 2 },
                        payload: EntryPayload::Membership(Membership::new_single(btreeset! {3,4,5})),
                    },
                ])
                .await?;

            let mem = store.last_membership_in_log(0).await?;

            assert!(mem.is_none());
        }

        tracing::info!("--- membership presents in log, smaller than last_applied, read from log");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: (1, 1).into(),
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {1,2,3})),
                }])
                .await?;

            let mem = store.last_membership_in_log(0).await?;
            let mem = mem.unwrap();
            assert_eq!(Membership::new_single(btreeset! {1, 2, 3}), mem.membership,);

            let mem = store.last_membership_in_log(1).await?;
            let mem = mem.unwrap();
            assert_eq!(Membership::new_single(btreeset! {1, 2, 3}), mem.membership,);

            let mem = store.last_membership_in_log(2).await?;
            assert!(mem.is_none());
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read from log");
        {
            store
                .append_to_log(&[
                    &Entry {
                        log_id: LogId { term: 1, index: 3 },
                        payload: EntryPayload::Membership(Membership::new_single(btreeset! {7,8,9})),
                    },
                    &Entry {
                        log_id: LogId { term: 1, index: 4 },
                        payload: EntryPayload::Blank,
                    },
                ])
                .await?;

            let mem = store.last_membership_in_log(0).await?;
            let mem = mem.unwrap();

            assert_eq!(Membership::new_single(btreeset! {7,8,9},), mem.membership,);
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read from log but since_index is greater than the last");
        {
            let mem = store.last_membership_in_log(4).await?;
            assert!(mem.is_none());
        }

        Ok(())
    }

    pub async fn get_membership_initial(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let membership = store.get_membership().await?;

        assert!(membership.is_none());

        Ok(())
    }

    pub async fn get_membership_from_log_and_sm(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        tracing::info!("--- no log, read membership from state machine");
        {
            store
                .apply_to_state_machine(&[
                    &Entry {
                        log_id: LogId { term: 1, index: 1 },
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 1, index: 2 },
                        payload: EntryPayload::Membership(Membership::new_single(btreeset! {3,4,5})),
                    },
                ])
                .await?;

            let mem = store.get_membership().await?;
            let mem = mem.unwrap();

            assert_eq!(Membership::new_single(btreeset! {3,4,5}), mem.membership,);
        }

        tracing::info!("--- membership presents in log, but smaller than last_applied, read from state machine");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: (1, 1).into(),
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {1,2,3})),
                }])
                .await?;

            let mem = store.get_membership().await?;

            let mem = mem.unwrap();

            assert_eq!(Membership::new_single(btreeset! {3, 4, 5}), mem.membership,);
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read from log");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: LogId { term: 1, index: 3 },
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {7,8,9})),
                }])
                .await?;

            let mem = store.get_membership().await?;

            let mem = mem.unwrap();

            assert_eq!(Membership::new_single(btreeset! {7,8,9},), mem.membership,);
        }

        Ok(())
    }

    pub async fn get_initial_state_without_init(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let initial = store.get_initial_state().await?;
        assert_eq!(InitialState::default(), initial, "uninitialized state");
        Ok(())
    }

    pub async fn get_initial_state_with_state(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;
        Self::default_hard_state(&store).await?;

        store
            .append_to_log(&[&Entry {
                log_id: (3, 2).into(),
                payload: EntryPayload::Blank,
            }])
            .await?;

        store
            .apply_to_state_machine(&[&Entry {
                log_id: LogId { term: 3, index: 1 },
                payload: EntryPayload::Blank,
            }])
            .await?;

        let initial = store.get_initial_state().await?;

        assert_eq!(
            initial.last_log_id,
            Some(LogId { term: 3, index: 2 }),
            "state machine has higher log"
        );
        assert_eq!(
            initial.last_applied,
            Some(LogId { term: 3, index: 1 }),
            "unexpected value for last applied log"
        );
        assert_eq!(
            HardState {
                current_term: 1,
                voted_for: Some(NODE_ID),
            },
            initial.hard_state,
            "unexpected value for default hard state"
        );
        Ok(())
    }

    pub async fn get_initial_state_membership_from_log_and_sm(builder: &B) -> anyhow::Result<()> {
        // It should never return membership from logs that are included in state machine present.

        let store = builder.build().await;
        Self::default_hard_state(&store).await?;

        // copy the test from get_membership_config

        tracing::info!("--- no log, read membership from state machine");
        {
            store
                .apply_to_state_machine(&[
                    &Entry {
                        log_id: LogId { term: 1, index: 1 },
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 1, index: 2 },
                        payload: EntryPayload::Membership(Membership::new_single(btreeset! {3,4,5})),
                    },
                ])
                .await?;

            let initial = store.get_initial_state().await?;

            assert_eq!(
                Membership::new_single(btreeset! {3,4,5}),
                initial.last_membership.unwrap().membership,
            );
        }

        tracing::info!("--- membership presents in log, but smaller than last_applied, read from state machine");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: (1, 1).into(),
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {1,2,3})),
                }])
                .await?;

            let initial = store.get_initial_state().await?;

            assert_eq!(
                Membership::new_single(btreeset! {3,4,5}),
                initial.last_membership.unwrap().membership,
            );
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read from log");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: LogId { term: 1, index: 3 },
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {1,2,3})),
                }])
                .await?;

            let initial = store.get_initial_state().await?;

            assert_eq!(
                Membership::new_single(btreeset! {1,2,3}),
                initial.last_membership.unwrap().membership,
            );
        }

        Ok(())
    }

    pub async fn get_initial_state_last_log_gt_sm(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;
        Self::default_hard_state(&store).await?;

        store
            .append_to_log(&[&Entry {
                log_id: (2, 1).into(),
                payload: EntryPayload::Blank,
            }])
            .await?;

        store
            .apply_to_state_machine(&[
                &Entry {
                    log_id: LogId { term: 1, index: 1 },
                    payload: EntryPayload::Blank,
                },
                &Entry {
                    log_id: LogId { term: 1, index: 2 },
                    payload: EntryPayload::Blank,
                },
            ])
            .await?;

        let initial = store.get_initial_state().await?;

        assert_eq!(
            initial.last_log_id,
            Some(LogId { term: 2, index: 1 }),
            "state machine has higher log"
        );
        Ok(())
    }

    pub async fn get_initial_state_last_log_lt_sm(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;
        Self::default_hard_state(&store).await?;

        store
            .append_to_log(&[&Entry {
                log_id: (1, 2).into(),
                payload: EntryPayload::Blank,
            }])
            .await?;

        store
            .apply_to_state_machine(&[&Entry {
                log_id: LogId { term: 3, index: 1 },
                payload: EntryPayload::Blank,
            }])
            .await?;

        let initial = store.get_initial_state().await?;

        assert_eq!(
            initial.last_log_id,
            Some(LogId { term: 3, index: 1 }),
            "state machine has higher log"
        );
        Ok(())
    }

    pub async fn save_hard_state(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        store
            .save_hard_state(&HardState {
                current_term: 100,
                voted_for: Some(NODE_ID),
            })
            .await?;

        let got = store.read_hard_state().await?;

        assert_eq!(
            Some(HardState {
                current_term: 100,
                voted_for: Some(NODE_ID),
            }),
            got,
        );
        Ok(())
    }

    pub async fn get_log_entries(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;
        Self::feed_10_logs_vote_self(&store).await?;

        tracing::info!("--- get start == stop");
        {
            let logs = store.get_log_entries(3..3).await?;
            assert_eq!(logs.len(), 0, "expected no logs to be returned");
        }

        tracing::info!("--- get start < stop");
        {
            let logs = store.get_log_entries(5..7).await?;

            assert_eq!(logs.len(), 2);
            assert_eq!(logs[0].log_id, (1, 5).into());
            assert_eq!(logs[1].log_id, (1, 6).into());
        }

        Ok(())
    }

    pub async fn try_get_log_entry(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;
        Self::feed_10_logs_vote_self(&store).await?;

        store.delete_log(0..=0).await?;

        let ent = store.try_get_log_entry(3).await?;
        assert_eq!(Some(LogId { term: 1, index: 3 }), ent.map(|x| x.log_id));

        let ent = store.try_get_log_entry(0).await?;
        assert_eq!(None, ent.map(|x| x.log_id));

        let ent = store.try_get_log_entry(11).await?;
        assert_eq!(None, ent.map(|x| x.log_id));

        Ok(())
    }

    pub async fn initial_logs(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let ent = store.try_get_log_entry(0).await?;
        assert!(ent.is_none(), "store initialized");

        Ok(())
    }

    pub async fn first_known_log_id(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let log_id = store.first_known_log_id().await?;
        assert_eq!(None, log_id, "store initialized");

        tracing::info!("--- returns the min id");
        {
            store.append_to_log(&[&blank(1, 1), &blank(1, 2)]).await?;

            store.delete_log(0..2).await?;

            // NOTE: it assumes non applied logs always exist.
            let log_id = store.first_known_log_id().await?;
            assert_eq!(Some(LogId::new(1, 2)), log_id, "last_applied is None");

            store.apply_to_state_machine(&[&blank(1, 1)]).await?;
            let log_id = store.first_known_log_id().await?;
            assert_eq!(Some(LogId::new(1, 1)), log_id);

            store.apply_to_state_machine(&[&blank(1, 2)]).await?;
            let log_id = store.first_known_log_id().await?;
            assert_eq!(Some(LogId::new(1, 2)), log_id);

            store.apply_to_state_machine(&[&blank(1, 3)]).await?;
            let log_id = store.first_known_log_id().await?;
            assert_eq!(Some(LogId::new(1, 2)), log_id, "least id is in log");

            store.delete_log(0..3).await?;
            let log_id = store.first_known_log_id().await?;
            assert_eq!(Some(LogId::new(1, 3)), log_id, "no logs, returns last applied log id");
        }

        Ok(())
    }

    pub async fn get_log_state(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;
        let (first_log_id, last_log_id) = store.get_log_state().await?;
        assert_eq!(None, first_log_id, "store initialized");
        assert_eq!(None, last_log_id, "last_log_id is None when store initialized");

        tracing::info!("--- only logs");
        {
            store
                .append_to_log(&[
                    &Entry {
                        log_id: LogId { term: 1, index: 1 },
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 1, index: 2 },
                        payload: EntryPayload::Blank,
                    },
                ])
                .await?;

            let (first_log_id, last_log_id) = store.get_log_state().await?;
            assert_eq!(Some(LogId::new(1, 1)), first_log_id);
            assert_eq!(Some(LogId::new(1, 2)), last_log_id);
        }

        tracing::info!("--- last id in logs < last applied id in sm, only return the id in logs");
        {
            store
                .apply_to_state_machine(&[&Entry {
                    log_id: LogId { term: 1, index: 3 },
                    payload: EntryPayload::Blank,
                }])
                .await?;
            let (_, last_log_id) = store.get_log_state().await?;
            assert_eq!(Some(LogId::new(1, 2)), last_log_id);
        }

        tracing::info!("--- no logs, return default");
        {
            store.delete_log(..).await?;

            let (first_log_id, last_log_id) = store.get_log_state().await?;
            assert_eq!(None, first_log_id);
            assert_eq!(None, last_log_id);
        }

        Ok(())
    }

    pub async fn first_id_in_log(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let log_id = store.first_id_in_log().await?;
        assert!(log_id.is_none(), "store initialized");

        tracing::info!("--- only logs");
        {
            store
                .append_to_log(&[
                    &Entry {
                        log_id: LogId { term: 1, index: 1 },
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 1, index: 2 },
                        payload: EntryPayload::Blank,
                    },
                ])
                .await?;

            let log_id = store.first_id_in_log().await?;
            assert_eq!(Some(LogId::new(1, 1)), log_id);
        }

        tracing::info!("--- no logs, return default");
        {
            store.delete_log(..).await?;

            let log_id = store.first_id_in_log().await?;
            assert_eq!(None, log_id);
        }

        Ok(())
    }

    pub async fn last_id_in_log(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let log_id = store.last_id_in_log().await?;
        assert_eq!(None, log_id);

        tracing::info!("--- only logs");
        {
            store
                .append_to_log(&[
                    &Entry {
                        log_id: LogId { term: 1, index: 1 },
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 1, index: 2 },
                        payload: EntryPayload::Blank,
                    },
                ])
                .await?;

            let log_id = store.last_id_in_log().await?;
            assert_eq!(Some(LogId { term: 1, index: 2 }), log_id);
        }

        tracing::info!("--- last id in logs < last applied id in sm, only return the id in logs");
        {
            store
                .apply_to_state_machine(&[&Entry {
                    log_id: LogId { term: 1, index: 3 },
                    payload: EntryPayload::Blank,
                }])
                .await?;
            let log_id = store.last_id_in_log().await?;
            assert_eq!(Some(LogId { term: 1, index: 2 }), log_id);
        }

        tracing::info!("--- no logs, return default");
        {
            store.delete_log(..).await?;

            let log_id = store.last_id_in_log().await?;
            assert_eq!(None, log_id);
        }

        Ok(())
    }

    pub async fn last_applied_state(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let (applied, membership) = store.last_applied_state().await?;
        assert_eq!(None, applied);
        assert_eq!(None, membership);

        tracing::info!("--- with last_applied and last_membership");
        {
            store
                .apply_to_state_machine(&[&Entry {
                    log_id: LogId { term: 1, index: 3 },
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {1,2})),
                }])
                .await?;

            let (applied, membership) = store.last_applied_state().await?;
            assert_eq!(Some(LogId { term: 1, index: 3 }), applied);
            assert_eq!(
                Some(EffectiveMembership {
                    log_id: LogId { term: 1, index: 3 },
                    membership: Membership::new_single(btreeset! {1,2})
                }),
                membership
            );
        }

        tracing::info!("--- no logs, return default");
        {
            store
                .apply_to_state_machine(&[&Entry {
                    log_id: LogId { term: 1, index: 5 },
                    payload: EntryPayload::Blank,
                }])
                .await?;

            let (applied, membership) = store.last_applied_state().await?;
            assert_eq!(Some(LogId { term: 1, index: 5 }), applied);
            assert_eq!(
                Some(EffectiveMembership {
                    log_id: LogId { term: 1, index: 3 },
                    membership: Membership::new_single(btreeset! {1,2})
                }),
                membership
            );
        }

        Ok(())
    }

    pub async fn delete_logs_from(builder: &B) -> anyhow::Result<()> {
        tracing::info!("--- delete start == stop");
        {
            let store = builder.build().await;
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_log(1..1).await?;

            let logs = store.get_log_entries(1..11).await?;
            assert_eq!(logs.len(), 10, "expected all (10) logs to be preserved");
        }

        tracing::info!("--- delete start < stop");
        {
            let store = builder.build().await;
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_log(..=0).await?;

            store.delete_log(1..4).await?;

            let logs = store.try_get_log_entries(0..100).await?;
            assert_eq!(logs.len(), 7);
            assert_eq!(logs[0].log_id.index, 4);
        }

        tracing::info!("--- delete start < large stop");
        {
            let store = builder.build().await;
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_log(..=0).await?;

            store.delete_log(1..1000).await?;
            let logs = store.try_get_log_entries(0..).await?;

            assert_eq!(logs.len(), 0);
        }

        tracing::info!("--- delete start, None");
        {
            let store = builder.build().await;
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_log(..=0).await?;

            store.delete_log(1..).await?;
            let logs = store.try_get_log_entries(0..100).await?;

            assert_eq!(logs.len(), 0);
        }

        Ok(())
    }

    pub async fn append_to_log(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;
        Self::feed_10_logs_vote_self(&store).await?;

        store.delete_log(..=0).await?;

        store
            .append_to_log(&[&Entry {
                log_id: (2, 10).into(),
                payload: EntryPayload::Blank,
            }])
            .await?;

        let l = store.try_get_log_entries(0..).await?.len();
        let last = store.try_get_log_entries(0..).await?.last().unwrap().clone();

        assert_eq!(l, 10, "expected 10 entries to exist in the log");
        assert_eq!(last.log_id, (2, 10).into(), "unexpected log id");
        Ok(())
    }

    // pub async fn apply_single(builder: &B) -> anyhow::Result<()> {
    //     let store = builder.build().await;
    //
    //     let entry = Entry {
    //         log_id: LogId { term: 3, index: 1 },
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
    //         Some(LogId { term: 3, index: 1 }),
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
    // pub async fn apply_multi(builder: &B) -> anyhow::Result<()> {
    //     let store = builder.build().await;
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
    //         (&LogId { term: 3, index: 1 }, &req0),
    //         (&LogId { term: 3, index: 2 }, &req1),
    //         (&LogId { term: 3, index: 3 }, &req2),
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
    //         Some(LogId { term: 3, index: 3 }),
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

    pub async fn feed_10_logs_vote_self(sto: &S) -> anyhow::Result<()> {
        sto.append_to_log(&[&blank(0, 0)]).await?;

        for i in 1..=10 {
            sto.append_to_log(&[&Entry {
                log_id: (1, i).into(),
                payload: EntryPayload::Blank,
            }])
            .await?;
        }

        Self::default_hard_state(sto).await?;

        Ok(())
    }

    pub async fn default_hard_state(sto: &S) -> anyhow::Result<()> {
        sto.save_hard_state(&HardState {
            current_term: 1,
            voted_for: Some(NODE_ID),
        })
        .await?;

        Ok(())
    }
}

// Defensive test:
// If a RaftStore impl support defensive check, enable it and check if it returns errors when abnormal input is seen.
// A RaftStore with defensive check is able to expose bugs in raft core.
impl<D, R, S, B> Suite<D, R, S, B>
where
    D: AppData + Debug,
    R: AppDataResponse + Debug,
    S: RaftStorage<D, R>,
    B: StoreBuilder<D, R, S>,
{
    pub fn test_store_defensive(builder: &B) -> anyhow::Result<()> {
        println!("---2");
        run_fut(Suite::df_get_membership_config_dirty_log(builder))?;
        run_fut(Suite::df_get_initial_state_dirty_log(builder))?;
        run_fut(Suite::df_save_hard_state_ascending(builder))?;
        run_fut(Suite::df_get_log_entries(builder))?;
        run_fut(Suite::df_delete_logs_from_nonempty_range(builder))?;
        run_fut(Suite::df_append_to_log_nonempty_input(builder))?;
        run_fut(Suite::df_append_to_log_nonconsecutive_input(builder))?;
        run_fut(Suite::df_append_to_log_eq_last_plus_one(builder))?;
        run_fut(Suite::df_append_to_log_eq_last_applied_plus_one(builder))?;
        run_fut(Suite::df_append_to_log_gt_last_log_id(builder))?;
        run_fut(Suite::df_append_to_log_gt_last_applied_id(builder))?;
        run_fut(Suite::df_apply_nonempty_input(builder))?;
        run_fut(Suite::df_apply_index_eq_last_applied_plus_one(builder))?;
        run_fut(Suite::df_apply_gt_last_applied_id(builder))?;

        Ok(())
    }

    pub async fn df_get_membership_config_dirty_log(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        tracing::info!("--- dirty log: log.index > last_applied.index && log < last_applied");
        {
            store
                .append_to_log(&[
                    &Entry {
                        log_id: LogId::new(0, 0),
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 1, index: 1 },
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 1, index: 2 },
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 1, index: 3 },
                        payload: EntryPayload::Membership(Membership::new_single(btreeset! {1,2,3})),
                    },
                ])
                .await?;
            store
                .apply_to_state_machine(&[
                    &Entry {
                        log_id: LogId { term: 2, index: 0 },
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 2, index: 1 },
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 2, index: 2 },
                        payload: EntryPayload::Membership(Membership::new_single(btreeset! {3,4,5})),
                    },
                ])
                .await?;

            let res = store.get_membership().await;

            let e = res.unwrap_err().into_defensive().unwrap();
            assert!(matches!(e, DefensiveError {
                subject: ErrorSubject::Log(LogId { term: 1, index: 3 }),
                violation: Violation::DirtyLog {
                    higher_index_log_id: LogId { term: 1, index: 3 },
                    lower_index_log_id: LogId { term: 2, index: 2 },
                },
                ..
            }))
        }

        Ok(())
    }

    pub async fn df_get_initial_state_dirty_log(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        tracing::info!("--- dirty log: log.index > last_applied.index && log < last_applied");
        {
            store
                .append_to_log(&[&blank(0, 0), &blank(1, 1), &blank(1, 2), &Entry {
                    log_id: LogId { term: 1, index: 3 },
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {1,2,3})),
                }])
                .await?;

            store
                .apply_to_state_machine(&[&blank(0, 0), &blank(2, 1), &Entry {
                    log_id: LogId { term: 2, index: 2 },
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {3,4,5})),
                }])
                .await?;

            let state = store.get_initial_state().await;
            let e = state.unwrap_err().into_defensive().unwrap();

            assert!(matches!(e, DefensiveError {
                subject: ErrorSubject::Log(LogId { term: 1, index: 3 }),
                violation: Violation::DirtyLog {
                    higher_index_log_id: LogId { term: 1, index: 3 },
                    lower_index_log_id: LogId { term: 2, index: 2 },
                },
                ..
            }))
        }

        Ok(())
    }

    pub async fn df_save_hard_state_ascending(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        store
            .save_hard_state(&HardState {
                current_term: 10,
                voted_for: Some(NODE_ID),
            })
            .await?;

        tracing::info!("--- lower term is rejected");
        {
            let res = store
                .save_hard_state(&HardState {
                    current_term: 9,
                    voted_for: Some(NODE_ID),
                })
                .await;

            let e = res.unwrap_err().into_defensive().unwrap();
            assert!(matches!(e, DefensiveError {
                subject: ErrorSubject::HardState,
                violation: Violation::TermNotAscending { curr: 10, to: 9 },
                ..
            }));

            let hs = store.read_hard_state().await?;

            assert_eq!(
                Some(HardState {
                    current_term: 10,
                    voted_for: Some(NODE_ID),
                }),
                hs,
            );
        }

        tracing::info!("--- same term can not reset to None");
        {
            let res = store
                .save_hard_state(&HardState {
                    current_term: 10,
                    voted_for: None,
                })
                .await;

            let e = res.unwrap_err().into_defensive().unwrap();
            assert!(matches!(e, DefensiveError {
                subject: ErrorSubject::HardState,
                violation: Violation::VotedForChanged {
                    curr: HardState {
                        current_term: 10,
                        voted_for: Some(NODE_ID)
                    },
                    to: HardState {
                        current_term: 10,
                        voted_for: None
                    }
                },
                ..
            }));

            let hs = store.read_hard_state().await?;

            assert_eq!(
                Some(HardState {
                    current_term: 10,
                    voted_for: Some(NODE_ID),
                }),
                hs,
            );
        }

        tracing::info!("--- same term can not change voted_for");
        {
            let res = store
                .save_hard_state(&HardState {
                    current_term: 10,
                    voted_for: Some(1000),
                })
                .await;

            let e = res.unwrap_err().into_defensive().unwrap();
            assert!(matches!(e, DefensiveError {
                subject: ErrorSubject::HardState,
                violation: Violation::VotedForChanged {
                    curr: HardState {
                        current_term: 10,
                        voted_for: Some(NODE_ID)
                    },
                    to: HardState {
                        current_term: 10,
                        voted_for: Some(1000)
                    }
                },
                ..
            }));

            let hs = store.read_hard_state().await?;

            assert_eq!(
                Some(HardState {
                    current_term: 10,
                    voted_for: Some(NODE_ID),
                }),
                hs
            );
        }

        Ok(())
    }

    pub async fn df_get_log_entries(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;
        Self::feed_10_logs_vote_self(&store).await?;

        store.delete_log(..=0).await?;

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

    pub async fn df_delete_logs_from_nonempty_range(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;
        Self::feed_10_logs_vote_self(&store).await?;

        let res = store.delete_log(10..10).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(ErrorSubject::Logs, e.subject);
        assert_eq!(
            Violation::RangeEmpty {
                start: Some(10),
                end: Some(9),
            },
            e.violation
        );

        let res = store.delete_log(1..5).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(ErrorSubject::Logs, e.subject);
        assert_eq!(
            Violation::RangeNotHalfOpen {
                start: Bound::Included(1),
                end: Bound::Excluded(5),
            },
            e.violation
        );

        Ok(())
    }

    pub async fn df_append_to_log_nonempty_input(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let res = store.append_to_log(Vec::<&Entry<_>>::new().as_slice()).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(ErrorSubject::Logs, e.subject);
        assert_eq!(Violation::LogsEmpty, e.violation);

        Ok(())
    }

    pub async fn df_append_to_log_nonconsecutive_input(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let res = store
            .append_to_log(&[
                &Entry {
                    log_id: (1, 1).into(),
                    payload: EntryPayload::Blank,
                },
                &Entry {
                    log_id: (1, 3).into(),
                    payload: EntryPayload::Blank,
                },
            ])
            .await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(ErrorSubject::Logs, e.subject);
        assert_eq!(
            Violation::LogsNonConsecutive {
                prev: Some(LogId { term: 1, index: 1 }),
                next: LogId { term: 1, index: 3 },
            },
            e.violation
        );

        Ok(())
    }

    pub async fn df_append_to_log_eq_last_plus_one(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        tracing::info!("-- log_id <= last_applied");
        tracing::info!("-- nonconsecutive log");
        tracing::info!("-- overlapping log");

        store.append_to_log(&[&blank(0, 0), &blank(1, 1), &blank(1, 2)]).await?;

        store.apply_to_state_machine(&[&blank(0, 0), &blank(1, 1)]).await?;

        let res = store.append_to_log(&[&blank(3, 4)]).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(ErrorSubject::Log(LogId { term: 3, index: 4 }), e.subject);
        assert_eq!(
            Violation::LogsNonConsecutive {
                prev: Some(LogId { term: 1, index: 2 }),
                next: LogId { term: 3, index: 4 },
            },
            e.violation
        );

        Ok(())
    }

    pub async fn df_append_to_log_eq_last_applied_plus_one(builder: &B) -> anyhow::Result<()> {
        // last_log: 1,1
        // last_applied: 1,2
        // append_to_log: 1,4
        let store = builder.build().await;

        tracing::info!("-- log_id <= last_applied");
        tracing::info!("-- nonconsecutive log");
        tracing::info!("-- overlapping log");

        store.append_to_log(&[&blank(0, 0), &blank(1, 1), &blank(1, 2)]).await?;

        store.apply_to_state_machine(&[&blank(0, 0), &blank(1, 1), &blank(1, 2)]).await?;

        store.delete_log(1..).await?;

        let res = store
            .append_to_log(&[&Entry {
                log_id: (1, 4).into(),
                payload: EntryPayload::Blank,
            }])
            .await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(ErrorSubject::Log(LogId { term: 1, index: 4 }), e.subject);
        assert_eq!(
            Violation::LogsNonConsecutive {
                prev: Some(LogId { term: 1, index: 2 }),
                next: LogId { term: 1, index: 4 },
            },
            e.violation
        );

        Ok(())
    }

    pub async fn df_append_to_log_gt_last_log_id(builder: &B) -> anyhow::Result<()> {
        // last_log: 2,2
        // append_to_log: 1,3: index == last + 1 but term is lower
        let store = builder.build().await;

        store.append_to_log(&[&blank(0, 0), &blank(2, 1), &blank(2, 2)]).await?;

        let res = store.append_to_log(&[&blank(1, 3)]).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(ErrorSubject::Log(LogId { term: 1, index: 3 }), e.subject);
        assert_eq!(
            Violation::LogsNonConsecutive {
                prev: Some(LogId { term: 2, index: 2 }),
                next: LogId { term: 1, index: 3 },
            },
            e.violation
        );

        Ok(())
    }

    pub async fn df_append_to_log_gt_last_applied_id(builder: &B) -> anyhow::Result<()> {
        // last_log: 2,1
        // last_applied: 2,2
        // append_to_log: 1,3: index == last + 1 but term is lower
        let store = builder.build().await;

        store.append_to_log(&[&blank(0, 0), &blank(2, 1), &blank(2, 2)]).await?;

        store.apply_to_state_machine(&[&blank(0, 0), &blank(2, 1), &blank(2, 2)]).await?;

        store.delete_log(1..).await?;

        let res = store.append_to_log(&[&blank(1, 3)]).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(ErrorSubject::Log(LogId { term: 1, index: 3 }), e.subject);
        assert_eq!(
            Violation::LogsNonConsecutive {
                prev: Some(LogId { term: 2, index: 2 }),
                next: LogId { term: 1, index: 3 },
            },
            e.violation
        );

        Ok(())
    }

    pub async fn df_apply_nonempty_input(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let res = store.apply_to_state_machine(Vec::<&Entry<_>>::new().as_slice()).await;

        let e = res.unwrap_err().into_defensive().unwrap();
        assert_eq!(ErrorSubject::Logs, e.subject);
        assert_eq!(Violation::LogsEmpty, e.violation);

        Ok(())
    }

    pub async fn df_apply_index_eq_last_applied_plus_one(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let entry = blank(3, 1);

        store.apply_to_state_machine(&[&blank(0, 0), &entry]).await?;

        tracing::info!("--- re-apply 1th");
        {
            let res = store.apply_to_state_machine(&[&entry]).await;

            let e = res.unwrap_err().into_defensive().unwrap();
            assert_eq!(ErrorSubject::Apply(LogId { term: 3, index: 1 }), e.subject);
            assert_eq!(
                Violation::ApplyNonConsecutive {
                    prev: Some(LogId { term: 3, index: 1 }),
                    next: LogId { term: 3, index: 1 },
                },
                e.violation
            );
        }

        tracing::info!("--- apply 3rd when there is only 1st");
        {
            let entry = blank(3, 3);
            let res = store.apply_to_state_machine(&[&entry]).await;

            let e = res.unwrap_err().into_defensive().unwrap();
            assert_eq!(ErrorSubject::Apply(LogId { term: 3, index: 3 }), e.subject);
            assert_eq!(
                Violation::ApplyNonConsecutive {
                    prev: Some(LogId { term: 3, index: 1 }),
                    next: LogId { term: 3, index: 3 },
                },
                e.violation
            );
        }

        Ok(())
    }

    pub async fn df_apply_gt_last_applied_id(builder: &B) -> anyhow::Result<()> {
        let store = builder.build().await;

        let entry = blank(3, 1);

        store.apply_to_state_machine(&[&blank(0, 0), &entry]).await?;

        tracing::info!("--- next apply with last_index+1 but lower term");
        {
            let entry = blank(2, 2);
            let res = store.apply_to_state_machine(&[&entry]).await;
            assert!(res.is_err());

            let e = res.unwrap_err().into_defensive().unwrap();
            assert_eq!(ErrorSubject::Apply(LogId { term: 2, index: 2 }), e.subject);
            assert_eq!(
                Violation::ApplyNonConsecutive {
                    prev: Some(LogId { term: 3, index: 1 }),
                    next: LogId { term: 2, index: 2 },
                },
                e.violation
            );
        }

        Ok(())
    }
}

/// Create a blank log entry for test
fn blank<D: AppData>(term: u64, index: u64) -> Entry<D> {
    Entry {
        log_id: LogId::new(term, index),
        payload: EntryPayload::Blank,
    }
}

/// Block until a future is finished.
/// The future will be running in a clean tokio runtime, to prevent an unfinished task affecting the test.
pub fn run_fut<F>(f: F) -> anyhow::Result<()>
where F: Future<Output = anyhow::Result<()>> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(f)?;
    Ok(())
}
