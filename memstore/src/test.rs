use std::future::Future;
use std::marker::PhantomData;

use async_raft::raft::EntryConfigChange;
use async_raft::raft::EntryNormal;
use maplit::btreeset;

use super::*;

const NODE_ID: u64 = 0;

pub trait StoreBuilder<S>: Clone {
    fn new_store(&self, id: NodeId) -> S;
}

#[derive(Clone)]
struct MemStoreBuilder {}

impl StoreBuilder<MemStore> for MemStoreBuilder {
    fn new_store(&self, id: NodeId) -> MemStore {
        MemStore::new(id)
    }
}

#[test]
pub fn test_mem_store() -> Result<()> {
    Suite::test_store(&MemStoreBuilder {})?;

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
    B: StoreBuilder<S>,
{
    p: PhantomData<S>,
    f: PhantomData<B>,
}

impl<S, B> Suite<S, B>
where
    S: RaftStorageDebug<MemStoreStateMachine> + RaftStorage<ClientRequest, ClientResponse>,
    B: StoreBuilder<S>,
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
        let store = builder.new_store(NODE_ID);

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
        let store = builder.new_store(NODE_ID);

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
        let store = builder.new_store(NODE_ID);

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
        let store = builder.new_store(NODE_ID);
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

        let store = builder.new_store(NODE_ID);
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
        let store = builder.new_store(NODE_ID);
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
        let store = builder.new_store(NODE_ID);
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
        let store = builder.new_store(NODE_ID);

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
        let store = builder.new_store(NODE_ID);
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
            let store = builder.new_store(NODE_ID);
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_logs_from(10, Some(1)).await?;

            let logs = store.get_log_entries(1, 11).await?;
            assert_eq!(logs.len(), 10, "expected all (10) logs to be preserved");
        }

        tracing::info!("--- delete start == stop");
        {
            let store = builder.new_store(NODE_ID);
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_logs_from(1, Some(1)).await?;

            let logs = store.get_log_entries(1, 11).await?;
            assert_eq!(logs.len(), 10, "expected all (10) logs to be preserved");
        }

        tracing::info!("--- delete start < stop");
        {
            let store = builder.new_store(NODE_ID);
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_logs_from(1, Some(4)).await?;

            let logs = store.get_log_entries(0, 100).await?;
            assert_eq!(logs.len(), 7);
            assert_eq!(logs[0].log_id.index, 4);
        }

        tracing::info!("--- delete start < large stop");
        {
            let store = builder.new_store(NODE_ID);
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_logs_from(1, Some(1000)).await?;
            let logs = store.get_log_entries(0, 100).await?;

            assert_eq!(logs.len(), 0);
        }

        tracing::info!("--- delete start, None");
        {
            let store = builder.new_store(NODE_ID);
            Self::feed_10_logs_vote_self(&store).await?;

            store.delete_logs_from(1, None).await?;
            let logs = store.get_log_entries(0, 100).await?;

            assert_eq!(logs.len(), 0);
        }

        Ok(())
    }

    pub async fn append_to_log(builder: &B) -> Result<()> {
        let store = builder.new_store(NODE_ID);
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
        let store = builder.new_store(NODE_ID);

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
        let store = builder.new_store(NODE_ID);

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
