use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::LinearizerOption;
use openraft::ReadPolicy;
use openraft::ServerState;
use openraft::async_runtime::OneshotSender;
use openraft::impls::ProgressResponder;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::expect_timeout;
use crate::fixtures::ut_harness;

/// How long a node is observed to show that it does not apply an entry.
const NO_APPLY_WINDOW: Duration = Duration::from_millis(500);

/// Requests a relaxed read with `policy`.
fn relaxed(policy: ReadPolicy) -> LinearizerOption {
    LinearizerOption::from(policy).with_relaxed(true)
}

/// Without the opt-in, a relaxed request gets the standard barrier.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn relaxed_request_without_opt_in_waits_for_committed() -> Result<()> {
    for enabled in [None, Some(false)] {
        let config = Config {
            enable_relaxed_read: enabled,
            enable_tick: false,
            ..Default::default()
        };
        let mut router = RaftRouter::new(config)?;
        tracing::info!(?enabled, "--- initialize a leader and pause its apply worker");
        let (n0, release, log_index) = {
            let log_index = router.new_cluster([0], []).await?;
            let n0 = router.get_raft_handle(&0)?;
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
            (n0, release_tx, log_index)
        };

        tracing::info!(
            ?enabled,
            log_index,
            "--- a relaxed request waits for the committed write"
        );
        {
            let (responder, committed, completed) = ProgressResponder::new();
            n0.write(ClientRequest::make_request("pending", 1)).responder(responder).await?;
            let committed = TypeConfig::timeout(WAIT_TIMEOUT, committed).await??;
            assert_eq!(log_index + 1, committed.index());

            let mut read = Box::pin(n0.ensure_linearizable(relaxed(ReadPolicy::ReadIndex)));
            let polled = futures::poll!(read.as_mut());
            assert!(polled.is_pending(), "a relaxed request without the opt-in must wait");

            drop(release);
            TypeConfig::timeout(WAIT_TIMEOUT, completed).await???;
            let read_log_id = TypeConfig::timeout(WAIT_TIMEOUT, read).await??;
            assert_eq!(log_index + 1, read_log_id.index());
        }
    }
    Ok(())
}

/// A relaxed read observes a write that has responded.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn relaxed_read_observes_a_completed_write() -> Result<()> {
    let config = Config {
        enable_relaxed_read: Some(true),
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    tracing::info!("--- initialize a leader and complete a write");
    let (n0, sm) = {
        router.new_cluster([0], []).await?;
        router.client_request(0, "completed", 1).await?;
        let n0 = router.get_raft_handle(&0)?;
        let (_, sm) = router.get_storage_handle(&0)?;
        (n0, sm)
    };

    tracing::info!("--- after the relaxed barrier, the state machine holds the write");
    {
        n0.ensure_linearizable(relaxed(ReadPolicy::ReadIndex)).await?;
        let state = sm.get_state_machine().await;
        assert_eq!(Some(&"request-1".to_string()), state.client_status.get("completed"));
    }
    Ok(())
}

/// Relaxed reads skip a committed apply backlog only on the leader; followers apply and wait as
/// usual.
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
        let (n0, n1, sm, release, log_index) = {
            let log_index = router.new_cluster([0, 1, 2], []).await?;
            let n0 = router.get_raft_handle(&0)?;
            let n1 = router.get_raft_handle(&1)?;
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
            (n0, n1, sm, release_tx, log_index)
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
            "--- relaxed reads succeed, standard barriers wait, followers do not apply ahead"
        );
        let (ready, on_follower) = {
            for policy in [ReadPolicy::ReadIndex, ReadPolicy::LeaseRead] {
                let read = n0.ensure_linearizable(relaxed(policy));
                let read_log_id = TypeConfig::timeout(WAIT_TIMEOUT, read).await??;
                assert_eq!(
                    log_index + 1,
                    read_log_id.index(),
                    "the reported read log ID stays the transferable boundary"
                );
                let state = sm.get_state_machine().await;
                assert_eq!(None, state.client_status.get("pending"));
            }
            let linearizer = n0.get_read_linearizer(ReadPolicy::ReadIndex).await?;
            assert_eq!(log_index + 1, linearizer.read_log_id().index());
            let mut ready = Box::pin(linearizer.await_ready(&n0));
            let polled = futures::poll!(ready.as_mut());
            assert!(polled.is_pending());
            let linearizer = n0.get_local_read_linearizer(ReadPolicy::LeaseRead).await?;
            assert_eq!(log_index + 1, linearizer.read_log_id().index());
            for follower in [1, 2] {
                let wait = router.wait_with_timeout(&follower, NO_APPLY_WINDOW);
                let result = wait.applied_index(Some(log_index + 1), "follower applies ahead of the leader").await;
                expect_timeout(result)?;
            }
            let on_follower = n0.get_read_linearizer(relaxed(ReadPolicy::ReadIndex)).await?;
            let result = on_follower.clone().try_await_ready(&n1, Some(NO_APPLY_WINDOW)).await?;
            assert!(
                result.is_err(),
                "a relaxed linearizer on a follower waits for committed"
            );
            (ready, on_follower)
        };

        tracing::info!(local, log_index, "--- release the leader's apply; followers then apply");
        {
            drop(release);
            let response = TypeConfig::timeout(WAIT_TIMEOUT, completed).await???;
            assert_eq!(log_index + 1, response.log_id.index());
            TypeConfig::timeout(WAIT_TIMEOUT, ready).await??;
            router.wait_applied_at_least([1, 2], log_index + 1, "followers apply after the leader").await?;
            TypeConfig::timeout(WAIT_TIMEOUT, on_follower.await_ready(&n1)).await??;
        }
    }
    Ok(())
}

/// A read that outlives its leadership sees only entries that the next leader has applied.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn relaxed_read_after_step_down_sees_only_what_the_next_leader_applied() -> Result<()> {
    let config = Config {
        enable_relaxed_read: Some(true),
        enable_local_lease_read: Some(true),
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    tracing::info!("--- initialize a cluster and pass a relaxed barrier on node 0");
    let (n0, n1, sm0, sm1, log_index) = {
        let log_index = router.new_cluster([0, 1, 2], []).await?;
        let n0 = router.get_raft_handle(&0)?;
        let n1 = router.get_raft_handle(&1)?;
        let (_, sm0) = router.get_storage_handle(&0)?;
        let (_, sm1) = router.get_storage_handle(&1)?;
        n0.ensure_linearizable(relaxed(ReadPolicy::ReadIndex)).await?;
        (n0, n1, sm0, sm1, log_index)
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
        let mut no_op_read = Box::pin(n1.ensure_linearizable(relaxed(ReadPolicy::ReadIndex)));
        let polled = futures::poll!(no_op_read.as_mut());
        assert!(polled.is_pending(), "a read cannot skip the new no-op barrier");
        drop(release_tx);
        TypeConfig::timeout(WAIT_TIMEOUT, no_op_read).await??;
    }

    tracing::info!(log_index, "--- commit a write on node 1 while its apply is paused");
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
        (release_tx, completed)
    };

    tracing::info!(log_index, "--- node 0 cannot apply the write before node 1 does");
    {
        let wait = router.wait_with_timeout(&0, NO_APPLY_WINDOW);
        let result = wait.applied_index(Some(log_index + 2), "node 0 applies ahead of node 1").await;
        expect_timeout(result)?;
        let state = sm0.get_state_machine().await;
        assert_eq!(
            None,
            state.client_status.get("new-leader"),
            "the read authorized on node 0 must not see a write node 1 has not applied"
        );
        n1.ensure_linearizable(relaxed(ReadPolicy::ReadIndex)).await?;
        let state = sm1.get_state_machine().await;
        assert_eq!(None, state.client_status.get("new-leader"));
    }

    tracing::info!(log_index, "--- node 0 applies the write only after node 1 applied it");
    {
        drop(release_worker);
        TypeConfig::timeout(WAIT_TIMEOUT, completed).await???;
        router.wait(&0).applied_index(Some(log_index + 2), "node 0 applies after node 1").await?;
        let state = sm0.get_state_machine().await;
        assert_eq!(Some(&"request-1".to_string()), state.client_status.get("new-leader"));
    }
    Ok(())
}
