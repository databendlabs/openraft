use anyhow::Result;
use openraft::Config;
use openraft::ReadPolicy;
use openraft::ServerState;
use openraft::async_runtime::OneshotSender;
use openraft::errors::LinearizableReadError;
use openraft::errors::RaftError;
use openraft::errors::RelaxedReadError;
use openraft::impls::ProgressResponder;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Disabled reads and follower requests fail before invoking application queries.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn relaxed_reads_require_opt_in_and_leadership() -> Result<()> {
    for enabled in [None, Some(false), Some(true)] {
        let config = Config {
            enable_relaxed_read: enabled,
            enable_tick: false,
            ..Default::default()
        };
        let mut router = RaftRouter::new(config)?;
        tracing::info!(?enabled, "--- initialize a cluster and reject unauthorized queries");
        {
            router.new_cluster([0, 1], []).await?;
            for node in [0, 1] {
                if enabled == Some(true) && node == 0 {
                    continue;
                }
                let raft = router.get_raft_handle(&node)?;
                let result = raft
                    .read_relaxed(ReadPolicy::ReadIndex, || async {
                        panic!("an unauthorized read must not run its query");
                        #[allow(unreachable_code)]
                        (None, ())
                    })
                    .await;
                if enabled == Some(true) {
                    assert!(matches!(
                        result,
                        Err(RaftError::APIError(RelaxedReadError::LinearizableRead(
                            LinearizableReadError::ForwardToLeader(_)
                        )))
                    ));
                } else {
                    assert!(matches!(result, Err(RaftError::APIError(RelaxedReadError::Disabled))));
                }
            }
        }
    }
    Ok(())
}

/// Snapshot validation rejects absent, old, and foreign-leadership applied IDs.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn relaxed_reads_validate_the_sampled_snapshot() -> Result<()> {
    let config = Config {
        enable_relaxed_read: Some(true),
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    tracing::info!("--- initialize a leader and complete a write before reading");
    let (n0, sm, written) = {
        router.new_cluster([0], []).await?;
        let n0 = router.get_raft_handle(&0)?;
        let (_, sm) = router.get_storage_handle(&0)?;
        let response = router.client_request(0, "completed", 1).await?;
        (n0, sm, response.log_id)
    };

    tracing::info!(?written, "--- a relaxed read observes the completed local write");
    {
        let value = n0
            .read_relaxed(ReadPolicy::ReadIndex, || async {
                let state = sm.get_state_machine().await;
                (state.last_applied_log, state.client_status.get("completed").cloned())
            })
            .await?;
        assert_eq!(Some("request-1".to_string()), value);
    }

    tracing::info!(?written, "--- validate the actual query snapshot, not ready metrics");
    {
        let invalid = [
            None,
            Some(log_id(1, 0, 0)),
            #[cfg(not(feature = "single-term-leader"))]
            Some(log_id(1, 1, 100)),
            Some(log_id(2, 0, 100)),
        ];
        for applied in invalid {
            let result = n0.read_relaxed(ReadPolicy::ReadIndex, || async { (applied, "discard") }).await;
            let Err(RaftError::APIError(RelaxedReadError::InvalidSnapshot { expected, applied: got })) = result else {
                panic!("invalid snapshot must not expose the query result: {result:?}");
            };
            assert_eq!(log_id(1, 0, 1), expected);
            assert_eq!(applied, got);
        }
    }
    Ok(())
}

/// Relaxed reads skip a committed apply backlog; the existing barriers still wait for it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn relaxed_reads_skip_the_committed_tail() -> Result<()> {
    for local in [false, true] {
        let config = Config {
            enable_relaxed_read: Some(true),
            enable_local_lease_read: Some(local),
            enable_tick: false,
            ..Default::default()
        };
        let mut router = RaftRouter::new(config)?;
        tracing::info!(local, "--- initialize a leader and pause its apply worker");
        let (n0, sm, release, log_index) = {
            let log_index = router.new_cluster([0, 1, 2], []).await?;
            let n0 = router.get_raft_handle(&0)?;
            let (_, sm) = router.get_storage_handle(&0)?;
            let (started_tx, started_rx) = TypeConfig::oneshot();
            let (release_tx, release_rx) = TypeConfig::oneshot::<()>();
            n0.external_state_machine_request(move |_| {
                Box::pin(async move {
                    started_tx.send(()).unwrap();
                    let _ = release_rx.await;
                })
            })
            .await?;
            TypeConfig::timeout(WAIT_TIMEOUT, started_rx).await??;
            (n0, sm, release_tx, log_index)
        };

        tracing::info!(local, log_index, "--- commit a write without applying or replying");
        let completed = {
            let (responder, committed, completed) = ProgressResponder::new();
            n0.write(ClientRequest::make_request("pending", 1)).responder(responder).await?;
            let committed = TypeConfig::timeout(WAIT_TIMEOUT, committed).await??;
            assert_eq!(log_index + 1, committed.index());
            completed
        };

        tracing::info!(
            local,
            log_index,
            "--- relaxed reads succeed but transferable barriers wait"
        );
        {
            for policy in [ReadPolicy::ReadIndex, ReadPolicy::LeaseRead] {
                let read = n0.read_relaxed(policy, || async {
                    let state = sm.get_state_machine().await;
                    (state.last_applied_log, state.client_status.get("pending").cloned())
                });
                let value = TypeConfig::timeout(WAIT_TIMEOUT, read).await??;
                assert_eq!(None, value);
            }
            let linearizer = n0.get_read_linearizer(ReadPolicy::ReadIndex).await?;
            assert_eq!(log_index + 1, linearizer.read_log_id().index());
            let mut ready = Box::pin(linearizer.await_ready(&n0));
            let polled = futures::poll!(ready.as_mut());
            assert!(polled.is_pending());
            let linearizer = n0.get_local_read_linearizer(ReadPolicy::LeaseRead).await?;
            assert_eq!(log_index + 1, linearizer.read_log_id().index());

            drop(release);
            let response = TypeConfig::timeout(WAIT_TIMEOUT, completed).await???;
            assert_eq!(log_index + 1, response.log_id.index());
            TypeConfig::timeout(WAIT_TIMEOUT, ready).await??;
        }
    }
    Ok(())
}

/// A read authorized before step-down must not expose state applied for the next leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn relaxed_read_rejects_a_snapshot_from_the_next_leader() -> Result<()> {
    let config = Config {
        enable_relaxed_read: Some(true),
        enable_local_lease_read: Some(true),
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    tracing::info!("--- initialize a cluster and authorize a read on node 0");
    let (n0, n1, sm0, sm1, log_index, release_read, mut read) = {
        let log_index = router.new_cluster([0, 1, 2], []).await?;
        let n0 = router.get_raft_handle(&0)?;
        let n1 = router.get_raft_handle(&1)?;
        let (_, sm0) = router.get_storage_handle(&0)?;
        let (_, sm1) = router.get_storage_handle(&1)?;
        let (started_tx, started_rx) = TypeConfig::oneshot();
        let (release_tx, release_rx) = TypeConfig::oneshot::<()>();
        let raft = n0.clone();
        let sm = sm0.clone();
        let read = TypeConfig::spawn(async move {
            raft.read_relaxed(ReadPolicy::ReadIndex, || async move {
                started_tx.send(()).unwrap();
                let _ = release_rx.await;
                let state = sm.get_state_machine().await;
                (state.last_applied_log, state.client_status.get("new-leader").cloned())
            })
            .await
        });
        TypeConfig::timeout(WAIT_TIMEOUT, started_rx).await??;
        (n0, n1, sm0, sm1, log_index, release_tx, read)
    };

    tracing::info!(log_index, "--- transfer to node 1 while its no-op apply is paused");
    {
        let (started_tx, started_rx) = TypeConfig::oneshot();
        let (release_tx, release_rx) = TypeConfig::oneshot::<()>();
        n1.external_state_machine_request(move |_| {
            Box::pin(async move {
                started_tx.send(()).unwrap();
                let _ = release_rx.await;
            })
        })
        .await?;
        TypeConfig::timeout(WAIT_TIMEOUT, started_rx).await??;
        n0.trigger().transfer_leader(1).await?;
        router.wait(&1).state(ServerState::Leader, "node 1 leads").await?;
        router.wait(&1).committed_index(Some(log_index + 1), "new no-op committed").await?;
        router.wait(&0).state(ServerState::Follower, "node 0 stepped down").await?;
        let mut no_op_read = Box::pin(n1.read_relaxed(ReadPolicy::ReadIndex, || async {
            let state = sm1.get_state_machine().await;
            (state.last_applied_log, ())
        }));
        let polled = futures::poll!(no_op_read.as_mut());
        assert!(polled.is_pending(), "a read cannot skip the new no-op barrier");
        drop(release_tx);
        TypeConfig::timeout(WAIT_TIMEOUT, no_op_read).await??;
    }

    tracing::info!(log_index, "--- let the follower apply a write ahead of its new leader");
    let (release_worker, completed) = {
        let (started_tx, started_rx) = TypeConfig::oneshot();
        let (release_tx, release_rx) = TypeConfig::oneshot::<()>();
        n1.external_state_machine_request(move |_| {
            Box::pin(async move {
                started_tx.send(()).unwrap();
                let _ = release_rx.await;
            })
        })
        .await?;
        TypeConfig::timeout(WAIT_TIMEOUT, started_rx).await??;
        let (responder, committed, completed) = ProgressResponder::new();
        n1.write(ClientRequest::make_request("new-leader", 1)).responder(responder).await?;
        let committed = TypeConfig::timeout(WAIT_TIMEOUT, committed).await??;
        assert_eq!(log_index + 2, committed.index());
        router.wait(&0).applied_index(Some(log_index + 2), "follower applied new write").await?;
        let state = sm0.get_state_machine().await;
        assert_eq!(Some(&"request-1".to_string()), state.client_status.get("new-leader"));
        (release_tx, completed)
    };

    tracing::info!(
        log_index,
        "--- discard node 0's newer snapshot before answering the client"
    );
    {
        drop(release_read);
        let result = TypeConfig::timeout(WAIT_TIMEOUT, &mut read).await??;
        assert!(matches!(
            result,
            Err(RaftError::APIError(RelaxedReadError::InvalidSnapshot { .. }))
        ));

        let value = n1
            .read_relaxed(ReadPolicy::ReadIndex, || async {
                let state = sm1.get_state_machine().await;
                (state.last_applied_log, state.client_status.get("new-leader").cloned())
            })
            .await?;
        assert_eq!(
            None, value,
            "the new leader still lags; returning node 0's result would regress"
        );
        drop(release_worker);
        TypeConfig::timeout(WAIT_TIMEOUT, completed).await???;
    }
    Ok(())
}
