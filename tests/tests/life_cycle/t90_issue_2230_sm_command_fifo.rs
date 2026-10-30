use std::io;
use std::time::Duration;

use anyhow::Result;
use maplit::hashmap;
use openraft::Config;
use openraft::ServerState;
use openraft::StorageError;
use openraft::async_runtime::OneshotSender;
use openraft::errors::ClientWriteError;
use openraft::errors::Fatal;
use openraft::errors::RaftError;
use openraft::impls::ProgressResponder;
use openraft::raft::ClientWriteResponse;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::JoinHandleOf;
use openraft::type_config::alias::LogIdOf;
use openraft::type_config::alias::OneshotReceiverOf;
use openraft::type_config::alias::OneshotSenderOf;
use openraft_memstore::BlockOperation;
use openraft_memstore::ClientRequest;
use openraft_memstore::ClientResponse;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemRaft;
use crate::fixtures::MemStateMachine;
use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

type WriteTask = JoinHandleOf<
    TypeConfig,
    Result<ClientWriteResponse<TypeConfig>, RaftError<TypeConfig, ClientWriteError<TypeConfig>>>,
>;

const APPLY_ERROR: &str = "injected apply failure";

const BACKPRESSURE_TIMEOUT: Duration = Duration::from_millis(50);

/// The bounded mode resumes a pending send even when the worker frees room without a notification.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn bounded_send_resumes_without_notification() -> Result<()> {
    let config = Config {
        state_machine_channel_size: Some(1),
        notification_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- pause the worker and fill its only command slot with another gate"
    );
    let (started_1, release_1) = admit_gate(&n0).await?;
    started_1.await?;
    let (started_2, release_2) = admit_gate(&n0).await?;

    tracing::info!(
        log_index,
        "--- commit a write whose apply waits for command-channel room"
    );
    let (responder, committed, mut completed) = ProgressResponder::new();
    let request = ClientRequest::make_request("foo", 1);
    n0.client_write_ff(request, Some(responder)).await?;
    let committed = TypeConfig::timeout(WAIT_TIMEOUT, committed).await??;
    let expected = log_id(1, 0, log_index + 1);
    assert_eq!(expected, committed);

    tracing::info!(
        log_index,
        "--- bounded backpressure delays a state read while both gates remain held"
    );
    let read = n0.with_raft_state(|state| state.local_committed().cloned());
    let mut read = Box::pin(read);
    {
        let result = TypeConfig::timeout(BACKPRESSURE_TIMEOUT, read.as_mut()).await;
        let blocked = result.is_err();
        assert!(blocked, "the bounded channel keeps the core waiting on its send");
    }

    tracing::info!(log_index, "--- the first gate frees a slot without notifying the core");
    {
        drop(release_1);
        TypeConfig::timeout(WAIT_TIMEOUT, started_2).await??;
        let observed = TypeConfig::timeout(WAIT_TIMEOUT, read).await??;
        assert_eq!(Some(expected), observed);
        let status = futures::poll!(&mut completed);
        let pending = status.is_pending();
        assert!(pending, "the second gate still prevents applying the write");
    }

    tracing::info!(log_index, "--- release the second gate and check the write response");
    {
        drop(release_2);
        let response = TypeConfig::timeout(WAIT_TIMEOUT, completed).await???;
        assert_eq!(expected, response.log_id);
        assert_eq!(ClientResponse(None), response.data);
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }
    Ok(())
}

/// A bounded command send drains notifications while the worker waits for notification capacity.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn bounded_send_handles_full_notification_channel() -> Result<()> {
    let config = Config {
        state_machine_channel_size: Some(2),
        notification_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let mut log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let (_, sm) = router.get_storage_handle(&0)?;

    tracing::info!(log_index, "--- build a snapshot whose logs can be purged later");
    let first_snapshot = log_id(1, 0, log_index);
    {
        n0.trigger().snapshot().await?;
        router.wait(&0).snapshot(first_snapshot, "first snapshot built").await?;
    }

    tracing::info!(
        log_index,
        "--- pause the worker and fill both command slots with applies"
    );
    let (started, release) = admit_gate(&n0).await?;
    started.await?;
    let mut writes = vec![];
    for serial in 1..=2 {
        log_index += 1;
        let write = commit_write(&router, serial, log_index).await?;
        writes.push(write);
    }

    tracing::info!(
        log_index,
        "--- hold the core in a purge while the worker fills notifications"
    );
    let purge_gate = sm.block.pause(BlockOperation::PurgeLog);
    {
        n0.trigger().purge_log(first_snapshot.index()).await?;
        TypeConfig::timeout(WAIT_TIMEOUT, purge_gate.entered()).await?;
        drop(release);
        let responses = write_responses(writes).await?;
        let expected = vec![
            (log_id(1, 0, 2), ClientResponse(None)),
            (log_id(1, 0, 3), ClientResponse(Some("request-1".to_string()))),
        ];
        assert_eq!(expected, responses);
    }

    tracing::info!(
        log_index,
        "--- fill both command slots while the worker waits to report its second apply"
    );
    let observed = {
        n0.external_state_machine_request(|_| Box::pin(async {})).await?;
        let (observed_tx, observed_rx) = TypeConfig::oneshot();
        n0.external_state_machine_request(move |sm: &mut MemStateMachine| {
            Box::pin(async move {
                let state = sm.get_state_machine().await;
                observed_tx.send(state.last_applied_log).ok();
            })
        })
        .await?;
        observed_rx
    };

    tracing::info!(
        log_index,
        "--- queue a snapshot build whose send must wait for command-channel room"
    );
    n0.trigger().snapshot().await?;
    let read = n0.with_raft_state(|state| state.local_committed().cloned());
    let mut read = Box::pin(read);
    let status = futures::poll!(read.as_mut());
    let pending = status.is_pending();
    assert!(pending, "the core remains inside the purge");

    tracing::info!(
        log_index,
        "--- release the core; receiving notifications must unblock the snapshot send"
    );
    {
        purge_gate.release();
        let expected = log_id(1, 0, log_index);
        let committed = TypeConfig::timeout(WAIT_TIMEOUT, read).await??;
        assert_eq!(Some(expected), committed);
        let observed = TypeConfig::timeout(WAIT_TIMEOUT, observed).await??;
        assert_eq!(Some(expected), observed);
        router.wait(&0).snapshot(expected, "second snapshot built").await?;
        router.wait(&0).purged(Some(first_snapshot), "purge completed").await?;
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }
    Ok(())
}

/// A worker error stops the core and releases responders from queued and waiting commands.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn bounded_worker_error_stops_core() -> Result<()> {
    let config = Config {
        state_machine_channel_size: Some(1),
        notification_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let (_, sm) = router.get_storage_handle(&0)?;

    tracing::info!(
        log_index,
        "--- pause the worker and commit a queued write and a waiting write"
    );
    let (started, release) = admit_gate(&n0).await?;
    started.await?;
    let mut completions = vec![];
    for serial in 1..=2 {
        let (responder, committed, completed) = ProgressResponder::new();
        let request = ClientRequest::make_request("foo", serial);
        n0.client_write_ff(request, Some(responder)).await?;
        let committed = TypeConfig::timeout(WAIT_TIMEOUT, committed).await??;
        let expected = log_id(1, 0, log_index + serial);
        assert_eq!(expected, committed);
        completions.push(completed);
    }

    tracing::info!(
        log_index,
        "--- fail the queued apply and stop the core through its error or the closed command channel"
    );
    {
        let source = io::Error::other(APPLY_ERROR);
        sm.fail_next_apply(source);
        drop(release);
        let wait = n0.wait(Some(WAIT_TIMEOUT));
        let metrics = wait.state(ServerState::Shutdown, "worker error stops the core").await?;
        let source = io::Error::other(APPLY_ERROR);
        let source = TypeConfig::err_from_error(&source);
        let failed_log_id = log_id(1, 0, log_index + 1);
        let error = StorageError::apply(failed_log_id, source);
        let worker_error = Err(Fatal::StorageError(error));
        let source = TypeConfig::err_from_string("cannot send to sm::Worker");
        let error = StorageError::write_state_machine(source);
        let send_error = Err(Fatal::StorageError(error));
        let expected = [worker_error, send_error];
        let expected_error = expected.contains(&metrics.running_state);
        assert!(expected_error, "unexpected running state: {:?}", metrics.running_state);
        for completed in completions {
            let result = TypeConfig::timeout(WAIT_TIMEOUT, completed).await?;
            let cancelled = result.is_err();
            assert!(cancelled, "the failed worker supplied no write response");
        }
        let state = sm.get_state_machine().await;
        let expected = Some(log_id(1, 0, log_index));
        assert_eq!(expected, state.last_applied_log);
        let expected_status = hashmap! {};
        assert_eq!(expected_status, state.client_status);
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }
    Ok(())
}

/// Shutdown cancels a send waiting for bounded command-channel capacity without waiting on the
/// worker.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn bounded_send_does_not_block_shutdown() -> Result<()> {
    let config = Config {
        state_machine_channel_size: Some(1),
        notification_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- hold the worker and fill its only command slot");
    let (started_1, release_1) = admit_gate(&n0).await?;
    started_1.await?;
    let (started_2, release_2) = admit_gate(&n0).await?;
    let (responder, committed, completed) = ProgressResponder::new();
    let request = ClientRequest::make_request("foo", 1);
    n0.client_write_ff(request, Some(responder)).await?;
    let committed = TypeConfig::timeout(WAIT_TIMEOUT, committed).await??;
    let expected = log_id(1, 0, log_index + 1);
    assert_eq!(expected, committed);

    tracing::info!(
        log_index,
        "--- stop the core while the worker cannot receive the waiting command"
    );
    let node = TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    {
        let result = TypeConfig::timeout(WAIT_TIMEOUT, completed).await?;
        let cancelled = result.is_err();
        assert!(cancelled, "shutdown discards the command that was never sent");
        let request = n0.external_state_machine_request(|_| Box::pin(async {}));
        let result = TypeConfig::timeout(WAIT_TIMEOUT, request).await?;
        assert_eq!(Err(Fatal::Stopped), result);
    }

    tracing::info!(
        log_index,
        "--- release the worker to drain the admitted gate before exiting"
    );
    {
        drop(release_1);
        TypeConfig::timeout(WAIT_TIMEOUT, started_2).await??;
        drop(release_2);
        let state = node.state_machine.get_state_machine().await;
        let expected = Some(log_id(1, 0, log_index));
        assert_eq!(expected, state.last_applied_log);
        let expected_status = hashmap! {};
        assert_eq!(expected_status, state.client_status);
    }
    Ok(())
}

/// Sends the state-machine worker a function that stops it until `release` is dropped.
///
/// Returns `(started, release)` once the worker channel admits the function. `started` resolves
/// when the worker begins to run it.
async fn admit_gate(n: &MemRaft) -> Result<(OneshotReceiverOf<TypeConfig, ()>, OneshotSenderOf<TypeConfig, ()>)> {
    let (started_tx, started_rx) = TypeConfig::oneshot();
    let (release_tx, release_rx) = TypeConfig::oneshot::<()>();
    n.external_state_machine_request(move |_sm| {
        Box::pin(async move {
            started_tx.send(()).ok();
            release_rx.await.ok();
        })
    })
    .await?;
    Ok((started_rx, release_tx))
}

/// Spawns a client write and returns its task once the write commits at `log_index`.
///
/// Waiting for each commit gives every write its own apply command. The task finishes when the
/// write is applied or RaftCore stops.
async fn commit_write(router: &RaftRouter, serial: u64, log_index: u64) -> Result<WriteTask> {
    let n0 = router.get_raft_handle(&0)?;
    let request = ClientRequest::make_request("foo", serial);
    let write = TypeConfig::spawn(async move { n0.client_write(request).await });
    router.wait(&0).committed_index(Some(log_index), format!("write {} committed", serial)).await?;
    Ok(write)
}

/// Waits for each write task and returns the log id and response of each, in order.
async fn write_responses(writes: Vec<WriteTask>) -> Result<Vec<(LogIdOf<TypeConfig>, ClientResponse)>> {
    let mut responses = vec![];
    for write in writes {
        let result = TypeConfig::timeout(WAIT_TIMEOUT, write).await??;
        let response = result?;
        responses.push((response.log_id, response.data));
    }
    Ok(responses)
}
