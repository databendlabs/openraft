use std::future::Future;
use std::marker::PhantomData;

use async_raft::raft::EntryConfigChange;
use async_raft::raft::EntryNormal;
use async_trait::async_trait;
use maplit::btreeset;

use super::*;

const NODE_ID: u64 = 0;

#[async_trait]
pub trait StoreBuilder<D, R, S>: Send + Sync
where
    D: AppData,
    R: AppDataResponse,
    S: RaftStorage<D, R>,
{
    async fn new_store(&self, id: NodeId) -> S;
}

struct MemStoreBuilder {}

#[async_trait]
impl StoreBuilder<ClientRequest, ClientResponse, MemStore> for MemStoreBuilder {
    async fn new_store(&self, id: NodeId) -> MemStore {
        MemStore::new(id)
    }
}

struct DefensiveBuilder<D, R, S, B>
where
    D: AppData,
    R: AppDataResponse,
    S: RaftStorage<D, R>,
    B: StoreBuilder<D, R, S>,
{
    inner: B,
    d: PhantomData<D>,
    r: PhantomData<R>,
    s: PhantomData<S>,
}

#[async_trait]
impl<D, R, S, B> StoreBuilder<D, R, S> for DefensiveBuilder<D, R, S, B>
where
    D: AppData,
    R: AppDataResponse,
    S: RaftStorage<D, R>,
    B: StoreBuilder<D, R, S>,
{
    async fn new_store(&self, id: NodeId) -> S {
        let dsto = self.inner.new_store(id).await;
        let d = dsto.defensive(true).await;
        assert!(d, "inner must impl defensive check");
        dsto
    }
}

#[test]
pub fn test_mem_store() -> Result<()> {
    Suite::test_store(&MemStoreBuilder {})?;

    Ok(())
}

#[test]
pub fn test_mem_store_defensive() -> Result<()> {
    Suite::test_store_defensive(&DefensiveBuilder {
        inner: MemStoreBuilder {},
        d: std::marker::PhantomData,
        r: std::marker::PhantomData,
        s: std::marker::PhantomData,
    })?;

    Ok(())
}

/// Block until a future is finished.
/// The future will be running in a clean tokio runtime, to prevent an unfinished task affecting the test.
pub fn run_fut<F>(f: F) -> Result<()>
where F: Future<Output = anyhow::Result<()>> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(f)?;
    Ok(())
}

/// Test suite to ensure a `RaftStore` impl works as expected.
struct Suite<S, B>
where
    S: RaftStorageDebug<MemStoreStateMachine> + RaftStorage<ClientRequest, ClientResponse>,
    B: StoreBuilder<ClientRequest, ClientResponse, S>,
{
    p: PhantomData<S>,
    f: PhantomData<B>,
}

impl<S, B> Suite<S, B>
where
    S: RaftStorageDebug<MemStoreStateMachine> + RaftStorage<ClientRequest, ClientResponse>,
    B: StoreBuilder<ClientRequest, ClientResponse, S>,
{
    fn test_store(builder: &B) -> Result<()> {
        run_fut(Suite::get_membership_config_default(builder))?;
        run_fut(Suite::get_membership_config_from_log_and_sm(builder))?;
        run_fut(Suite::get_initial_state_default(builder))?;
        run_fut(Suite::get_initial_state_membership_from_log_and_sm(builder))?;
        run_fut(Suite::get_initial_state_with_state(builder))?;
        run_fut(Suite::get_initial_state_last_log_gt_sm(builder))?;
        run_fut(Suite::get_initial_state_last_log_lt_sm(builder))?;
        run_fut(Suite::save_hard_state(builder))?;
        run_fut(Suite::get_log_entries(builder))?;
        run_fut(Suite::delete_logs_from(builder))?;
        run_fut(Suite::append_to_log(builder))?;
        run_fut(Suite::apply_single(builder))?;
        run_fut(Suite::apply_multi(builder))?;

        Ok(())
    }

    pub async fn get_membership_config_default(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

        let membership = store.get_membership_config().await?;

        assert_eq!(
            MembershipConfig {
                members: btreeset! {NODE_ID},
                members_after_consensus: None,
            },
            membership,
        );

        Ok(())
    }

    pub async fn get_membership_config_from_log_and_sm(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

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
                        payload: EntryPayload::ConfigChange(EntryConfigChange {
                            membership: MembershipConfig {
                                members: btreeset! {3,4,5},
                                members_after_consensus: None,
                            },
                        }),
                    },
                ])
                .await?;

            let mem = store.get_membership_config().await?;

            assert_eq!(
                MembershipConfig {
                    members: btreeset! {3,4,5},
                    members_after_consensus: None,
                },
                mem,
            );
        }

        tracing::info!("--- membership presents in log, but smaller than last_applied, read from state machine");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: (1, 1).into(),
                    payload: EntryPayload::ConfigChange(EntryConfigChange {
                        membership: MembershipConfig {
                            members: btreeset! {1,2,3},
                            members_after_consensus: None,
                        },
                    }),
                }])
                .await?;

            let mem = store.get_membership_config().await?;

            assert_eq!(
                MembershipConfig {
                    members: btreeset! {3, 4, 5},
                    members_after_consensus: None,
                },
                mem,
            );
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read from log");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: LogId { term: 1, index: 3 },
                    payload: EntryPayload::ConfigChange(EntryConfigChange {
                        membership: MembershipConfig {
                            members: btreeset! {1,2,3},
                            members_after_consensus: None,
                        },
                    }),
                }])
                .await?;

            let mem = store.get_membership_config().await?;

            assert_eq!(
                MembershipConfig {
                    members: btreeset! {1,2,3},
                    members_after_consensus: None,
                },
                mem,
            );
        }

        Ok(())
    }

    pub async fn get_initial_state_default(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

        let expected_hs = HardState {
            current_term: 0,
            voted_for: None,
        };

        let initial = store.get_initial_state().await?;

        assert_eq!(
            initial.last_log_id,
            LogId { term: 0, index: 0 },
            "unexpected default value for last log"
        );
        assert_eq!(
            initial.last_applied_log,
            LogId { term: 0, index: 0 },
            "unexpected value for last applied log"
        );

        assert_eq!(
            MembershipConfig {
                members: btreeset! {NODE_ID},
                members_after_consensus: None,
            },
            initial.membership,
        );

        assert_eq!(
            initial.hard_state, expected_hs,
            "unexpected value for default hard state"
        );
        Ok(())
    }

    pub async fn get_initial_state_with_state(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;
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
            LogId { term: 3, index: 2 },
            "state machine has higher log"
        );
        assert_eq!(
            initial.last_applied_log,
            LogId { term: 3, index: 1 },
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

    pub async fn get_initial_state_membership_from_log_and_sm(builder: &B) -> Result<()> {
        // It should never return membership from logs that are included in state machine present.

        let store = builder.new_store(NODE_ID).await;
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
                        payload: EntryPayload::ConfigChange(EntryConfigChange {
                            membership: MembershipConfig {
                                members: btreeset! {3,4,5},
                                members_after_consensus: None,
                            },
                        }),
                    },
                ])
                .await?;

            let initial = store.get_initial_state().await?;

            assert_eq!(
                MembershipConfig {
                    members: btreeset! {3,4,5},
                    members_after_consensus: None,
                },
                initial.membership,
            );
        }

        tracing::info!("--- membership presents in log, but smaller than last_applied, read from state machine");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: (1, 1).into(),
                    payload: EntryPayload::ConfigChange(EntryConfigChange {
                        membership: MembershipConfig {
                            members: btreeset! {1,2,3},
                            members_after_consensus: None,
                        },
                    }),
                }])
                .await?;

            let initial = store.get_initial_state().await?;

            assert_eq!(
                MembershipConfig {
                    members: btreeset! {3, 4, 5},
                    members_after_consensus: None,
                },
                initial.membership,
            );
        }

        tracing::info!("--- membership presents in log and > sm.last_applied, read from log");
        {
            store
                .append_to_log(&[&Entry {
                    log_id: LogId { term: 1, index: 3 },
                    payload: EntryPayload::ConfigChange(EntryConfigChange {
                        membership: MembershipConfig {
                            members: btreeset! {1,2,3},
                            members_after_consensus: None,
                        },
                    }),
                }])
                .await?;

            let initial = store.get_initial_state().await?;

            assert_eq!(
                MembershipConfig {
                    members: btreeset! {1,2,3},
                    members_after_consensus: None,
                },
                initial.membership,
            );
        }

        Ok(())
    }

    pub async fn get_initial_state_last_log_gt_sm(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;
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
            LogId { term: 2, index: 1 },
            "state machine has higher log"
        );
        Ok(())
    }

    pub async fn get_initial_state_last_log_lt_sm(builder: &B) -> Result<()> {
        // TODO(xp): check membership: read from log first, then state machine then default.
        let store = builder.new_store(NODE_ID).await;
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
            LogId { term: 3, index: 1 },
            "state machine has higher log"
        );
        Ok(())
    }

    pub async fn save_hard_state(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

        store
            .save_hard_state(&HardState {
                current_term: 100,
                voted_for: Some(NODE_ID),
            })
            .await?;

        let post = store.get_initial_state().await?;

        assert_eq!(
            HardState {
                current_term: 100,
                voted_for: Some(NODE_ID),
            },
            post.hard_state,
        );
        Ok(())
    }

    pub async fn get_log_entries(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;
        Self::feed_10_logs_vote_self(&store).await?;

        tracing::info!("--- get start > stop");
        {
            let logs = store.get_log_entries(10, 1).await?;
            assert_eq!(logs.len(), 0, "expected no logs to be returned");
        }

        tracing::info!("--- get start == stop");
        {
            let logs = store.get_log_entries(10, 1).await?;
            assert_eq!(logs.len(), 0, "expected no logs to be returned");
        }

        tracing::info!("--- get start < stop");
        {
            let logs = store.get_log_entries(5, 7).await?;

            assert_eq!(logs.len(), 2);
            assert_eq!(logs[0].log_id, (1, 5).into());
            assert_eq!(logs[1].log_id, (1, 6).into());
        }

        Ok(())
    }

    pub async fn delete_logs_from(builder: &B) -> Result<()> {
        tracing::info!("--- delete start > stop");
        {
            let store = builder.new_store(NODE_ID).await;
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_logs_from(10, Some(1)).await?;

            let logs = store.get_log_entries(1, 11).await?;
            assert_eq!(logs.len(), 10, "expected all (10) logs to be preserved");
        }

        tracing::info!("--- delete start == stop");
        {
            let store = builder.new_store(NODE_ID).await;
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_logs_from(1, Some(1)).await?;

            let logs = store.get_log_entries(1, 11).await?;
            assert_eq!(logs.len(), 10, "expected all (10) logs to be preserved");
        }

        tracing::info!("--- delete start < stop");
        {
            let store = builder.new_store(NODE_ID).await;
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_logs_from(1, Some(4)).await?;

            let logs = store.get_log_entries(0, 100).await?;
            assert_eq!(logs.len(), 7);
            assert_eq!(logs[0].log_id.index, 4);
        }

        tracing::info!("--- delete start < large stop");
        {
            let store = builder.new_store(NODE_ID).await;
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_logs_from(1, Some(1000)).await?;
            let logs = store.get_log_entries(0, 100).await?;

            assert_eq!(logs.len(), 0);
        }

        tracing::info!("--- delete start, None");
        {
            let store = builder.new_store(NODE_ID).await;
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_logs_from(1, None).await?;
            let logs = store.get_log_entries(0, 100).await?;

            assert_eq!(logs.len(), 0);
        }

        Ok(())
    }

    pub async fn append_to_log(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;
        Self::feed_10_logs_vote_self(&store).await?;

        store
            .append_to_log(&[&Entry {
                log_id: (2, 10).into(),
                payload: EntryPayload::Blank,
            }])
            .await?;

        let l = store.get_log_entries(0, 10_000).await?.len();
        let last = store.get_log_entries(0, 10_000).await?.last().unwrap().clone();

        assert_eq!(l, 10, "expected 10 entries to exist in the log");
        assert_eq!(last.log_id, (2, 10).into(), "unexpected log id");
        Ok(())
    }

    pub async fn apply_single(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

        let entry = Entry {
            log_id: LogId { term: 3, index: 1 },

            payload: EntryPayload::Normal(EntryNormal {
                data: ClientRequest {
                    client: "0".into(),
                    serial: 0,
                    status: "lit".into(),
                },
            }),
        };

        store.apply_to_state_machine(&[&entry]).await?;
        let sm = store.get_state_machine().await;

        assert_eq!(
            sm.last_applied_log,
            LogId { term: 3, index: 1 },
            "expected last_applied_log to be 1, got {}",
            sm.last_applied_log
        );

        let client_serial =
            sm.client_serial_responses.get("0").expect("expected entry to exist in client_serial_responses");
        assert_eq!(client_serial, &(0, None), "unexpected client serial response");

        let client_status = sm.client_status.get("0").expect("expected entry to exist in client_status");
        assert_eq!(
            client_status, "lit",
            "expected client_status to be 'lit', got '{}'",
            client_status
        );
        Ok(())
    }

    pub async fn apply_multi(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

        let req0 = ClientRequest {
            client: "1".into(),
            serial: 0,
            status: "old".into(),
        };
        let req1 = ClientRequest {
            client: "1".into(),
            serial: 1,
            status: "new".into(),
        };
        let req2 = ClientRequest {
            client: "2".into(),
            serial: 0,
            status: "other".into(),
        };

        let entries = vec![
            (&LogId { term: 3, index: 1 }, &req0),
            (&LogId { term: 3, index: 2 }, &req1),
            (&LogId { term: 3, index: 3 }, &req2),
        ]
        .into_iter()
        .map(|(id, req)| Entry {
            log_id: *id,
            payload: EntryPayload::Normal(EntryNormal { data: req.clone() }),
        })
        .collect::<Vec<_>>();

        store.apply_to_state_machine(&entries.iter().collect::<Vec<_>>()).await?;
        let sm = store.get_state_machine().await;

        assert_eq!(
            sm.last_applied_log,
            LogId { term: 3, index: 3 },
            "expected last_applied_log to be 3, got {}",
            sm.last_applied_log
        );

        let client_serial1 = sm
            .client_serial_responses
            .get("1")
            .expect("expected entry to exist in client_serial_responses for client 1");
        assert_eq!(client_serial1.0, 1, "unexpected client serial response");
        assert_eq!(
            client_serial1.1,
            Some(String::from("old")),
            "unexpected client serial response"
        );

        let client_serial2 = sm
            .client_serial_responses
            .get("2")
            .expect("expected entry to exist in client_serial_responses for client 2");
        assert_eq!(client_serial2.0, 0, "unexpected client serial response");
        assert_eq!(client_serial2.1, None, "unexpected client serial response");

        let client_status1 = sm.client_status.get("1").expect("expected entry to exist in client_status for client 1");
        let client_status2 = sm.client_status.get("2").expect("expected entry to exist in client_status for client 2");
        assert_eq!(
            client_status1, "new",
            "expected client_status to be 'new', got '{}'",
            client_status1
        );
        assert_eq!(
            client_status2, "other",
            "expected client_status to be 'other', got '{}'",
            client_status2
        );
        Ok(())
    }

    pub async fn feed_10_logs_vote_self(sto: &S) -> anyhow::Result<()> {
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
impl<S, B> Suite<S, B>
where
    S: RaftStorageDebug<MemStoreStateMachine> + RaftStorage<ClientRequest, ClientResponse>,
    B: StoreBuilder<ClientRequest, ClientResponse, S>,
{
    fn test_store_defensive(builder: &B) -> Result<()> {
        run_fut(Suite::df_get_membership_config_dirty_log(builder))?;
        run_fut(Suite::df_get_initial_state_dirty_log(builder))?;
        run_fut(Suite::df_save_hard_state_ascending(builder))?;
        run_fut(Suite::df_delete_logs_from(builder))?;
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

    pub async fn df_get_membership_config_dirty_log(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

        tracing::info!("--- dirty log: log.index > last_applied.index && log < last_applied");
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
                    &Entry {
                        log_id: LogId { term: 1, index: 3 },
                        payload: EntryPayload::ConfigChange(EntryConfigChange {
                            membership: MembershipConfig {
                                members: btreeset! {1,2,3},
                                members_after_consensus: None,
                            },
                        }),
                    },
                ])
                .await?;
            store
                .apply_to_state_machine(&[
                    &Entry {
                        log_id: LogId { term: 2, index: 1 },
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 2, index: 2 },
                        payload: EntryPayload::ConfigChange(EntryConfigChange {
                            membership: MembershipConfig {
                                members: btreeset! {3,4,5},
                                members_after_consensus: None,
                            },
                        }),
                    },
                ])
                .await?;

            let mem = store.get_membership_config().await;
            assert!(mem.is_err());
        }

        Ok(())
    }

    pub async fn df_get_initial_state_dirty_log(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

        tracing::info!("--- dirty log: log.index > last_applied.index && log < last_applied");
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
                    &Entry {
                        log_id: LogId { term: 1, index: 3 },
                        payload: EntryPayload::ConfigChange(EntryConfigChange {
                            membership: MembershipConfig {
                                members: btreeset! {1,2,3},
                                members_after_consensus: None,
                            },
                        }),
                    },
                ])
                .await?;

            store
                .apply_to_state_machine(&[
                    &Entry {
                        log_id: LogId { term: 2, index: 1 },
                        payload: EntryPayload::Blank,
                    },
                    &Entry {
                        log_id: LogId { term: 2, index: 2 },
                        payload: EntryPayload::ConfigChange(EntryConfigChange {
                            membership: MembershipConfig {
                                members: btreeset! {3,4,5},
                                members_after_consensus: None,
                            },
                        }),
                    },
                ])
                .await?;

            let state = store.get_initial_state().await;
            assert!(state.is_err());
        }

        Ok(())
    }

    pub async fn df_save_hard_state_ascending(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

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

            assert!(res.is_err());

            let state = store.get_initial_state().await?;

            assert_eq!(
                HardState {
                    current_term: 10,
                    voted_for: Some(NODE_ID),
                },
                state.hard_state,
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

            assert!(res.is_err());

            let state = store.get_initial_state().await?;

            assert_eq!(
                HardState {
                    current_term: 10,
                    voted_for: Some(NODE_ID),
                },
                state.hard_state,
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

            assert!(res.is_err());

            let state = store.get_initial_state().await?;

            assert_eq!(
                HardState {
                    current_term: 10,
                    voted_for: Some(NODE_ID),
                },
                state.hard_state,
            );
        }

        Ok(())
    }

    pub async fn df_delete_logs_from(_builder: &B) -> Result<()> {
        // TODO(xp): what should we test about this?
        Ok(())
    }

    pub async fn df_append_to_log_nonempty_input(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

        let res = store.append_to_log(Vec::<&Entry<_>>::new().as_slice()).await;
        assert!(res.is_err());

        Ok(())
    }

    pub async fn df_append_to_log_nonconsecutive_input(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

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
        assert!(res.is_err());

        Ok(())
    }

    pub async fn df_append_to_log_eq_last_plus_one(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

        tracing::info!("-- log_id <= last_applied");
        tracing::info!("-- nonconsecutive log");
        tracing::info!("-- overlapping log");

        store
            .append_to_log(&[
                &Entry {
                    log_id: (1, 1).into(),
                    payload: EntryPayload::Blank,
                },
                &Entry {
                    log_id: (1, 2).into(),
                    payload: EntryPayload::Blank,
                },
            ])
            .await?;

        store
            .apply_to_state_machine(&[&Entry {
                log_id: LogId { term: 1, index: 1 },
                payload: EntryPayload::Blank,
            }])
            .await?;

        let res = store
            .append_to_log(&[&Entry {
                log_id: (3, 4).into(),
                payload: EntryPayload::Blank,
            }])
            .await;

        assert!(res.is_err());

        Ok(())
    }

    pub async fn df_append_to_log_eq_last_applied_plus_one(builder: &B) -> Result<()> {
        // last_log: 1,1
        // last_applied: 1,2
        // append_to_log: 1,4
        let store = builder.new_store(NODE_ID).await;

        tracing::info!("-- log_id <= last_applied");
        tracing::info!("-- nonconsecutive log");
        tracing::info!("-- overlapping log");

        store
            .append_to_log(&[
                &Entry {
                    log_id: (1, 1).into(),
                    payload: EntryPayload::Blank,
                },
                &Entry {
                    log_id: (1, 2).into(),
                    payload: EntryPayload::Blank,
                },
            ])
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

        store.delete_logs_from(1, Some(2)).await?;

        let res = store
            .append_to_log(&[&Entry {
                log_id: (1, 4).into(),
                payload: EntryPayload::Blank,
            }])
            .await;

        assert!(res.is_err());

        Ok(())
    }

    pub async fn df_append_to_log_gt_last_log_id(builder: &B) -> Result<()> {
        // last_log: 2,2
        // append_to_log: 1,3: index == last + 1 but term is lower
        let store = builder.new_store(NODE_ID).await;

        store
            .append_to_log(&[
                &Entry {
                    log_id: (2, 1).into(),
                    payload: EntryPayload::Blank,
                },
                &Entry {
                    log_id: (2, 2).into(),
                    payload: EntryPayload::Blank,
                },
            ])
            .await?;

        let res = store
            .append_to_log(&[&Entry {
                log_id: (1, 3).into(),
                payload: EntryPayload::Blank,
            }])
            .await;

        assert!(res.is_err());

        Ok(())
    }

    pub async fn df_append_to_log_gt_last_applied_id(builder: &B) -> Result<()> {
        // last_log: 2,1
        // last_applied: 2,2
        // append_to_log: 1,3: index == last + 1 but term is lower
        let store = builder.new_store(NODE_ID).await;

        store
            .append_to_log(&[
                &Entry {
                    log_id: (2, 1).into(),
                    payload: EntryPayload::Blank,
                },
                &Entry {
                    log_id: (2, 2).into(),
                    payload: EntryPayload::Blank,
                },
            ])
            .await?;

        store
            .apply_to_state_machine(&[
                &Entry {
                    log_id: LogId { term: 2, index: 1 },
                    payload: EntryPayload::Blank,
                },
                &Entry {
                    log_id: LogId { term: 2, index: 2 },
                    payload: EntryPayload::Blank,
                },
            ])
            .await?;

        store.delete_logs_from(1, Some(2)).await?;

        let res = store
            .append_to_log(&[&Entry {
                log_id: (1, 3).into(),
                payload: EntryPayload::Blank,
            }])
            .await;

        assert!(res.is_err());

        Ok(())
    }

    pub async fn df_apply_nonempty_input(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

        let res = store.apply_to_state_machine(Vec::<&Entry<_>>::new().as_slice()).await;
        assert!(res.is_err());

        Ok(())
    }

    pub async fn df_apply_index_eq_last_applied_plus_one(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

        let entry = Entry {
            log_id: LogId { term: 3, index: 1 },

            payload: EntryPayload::Normal(EntryNormal {
                data: ClientRequest {
                    client: "0".into(),
                    serial: 0,
                    status: "lit".into(),
                },
            }),
        };

        store.apply_to_state_machine(&[&entry]).await?;

        tracing::info!("--- re-apply 1th");
        {
            let res = store.apply_to_state_machine(&[&entry]).await;
            assert!(res.is_err());
        }

        tracing::info!("--- apply 3rd when there is only 1st");
        {
            let entry = Entry {
                log_id: LogId { term: 3, index: 3 },

                payload: EntryPayload::Normal(EntryNormal {
                    data: ClientRequest {
                        client: "0".into(),
                        serial: 0,
                        status: "lit".into(),
                    },
                }),
            };
            let res = store.apply_to_state_machine(&[&entry]).await;
            assert!(res.is_err());
        }

        Ok(())
    }

    pub async fn df_apply_gt_last_applied_id(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID).await;

        let entry = Entry {
            log_id: LogId { term: 3, index: 1 },
            payload: EntryPayload::Blank,
        };

        store.apply_to_state_machine(&[&entry]).await?;

        tracing::info!("--- next apply with last_index+1 but lower term");
        {
            let entry = Entry {
                log_id: LogId { term: 2, index: 2 },
                payload: EntryPayload::Blank,
            };
            let res = store.apply_to_state_machine(&[&entry]).await;
            assert!(res.is_err());
        }

        Ok(())
    }
}
