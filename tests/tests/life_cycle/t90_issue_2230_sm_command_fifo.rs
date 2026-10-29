use std::io;
use std::time::Duration;

use anyhow::Result;
use maplit::hashmap;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::ReadPolicy;
use openraft::ServerState;
use openraft::StorageError;
use openraft::Vote;
use openraft::async_runtime::OneshotSender;
use openraft::errors::ClientWriteError;
use openraft::errors::Fatal;
use openraft::errors::RaftError;
use openraft::impls::ProgressResponder;
use openraft::raft::ClientWriteResponse;
use openraft::raft::VoteRequest;
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

const BUFFERED_WRITES: u64 = 8;
const BACKPRESSURE_TIMEOUT: Duration = Duration::from_millis(50);

/// Snapshot reads and external functions retain receive order behind buffered applies.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_reads_follow_buffered_commands() -> Result<()> {
    let config = Config {
        enable_unbounded_state_machine_buffer: true,
        state_machine_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    tracing::info!("--- bring up node 0 and build its first snapshot");
    let mut log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    {
        n0.trigger().snapshot().await?;
        let snapshot_log_id = log_id(1, 0, log_index);
        router.wait(&0).snapshot(snapshot_log_id, "first snapshot built").await?;
    }
    tracing::info!(log_index, "--- pause the worker and buffer three applies");
    let (started, release) = admit_gate(&n0).await?;
    started.await?;
    let mut writes = vec![];
    for serial in 1..=3 {
        log_index += 1;
        let write = commit_write(&router, serial, log_index).await?;
        writes.push(write);
    }
    tracing::info!(log_index, "--- queue snapshot removal before a snapshot read");
    {
        n0.external_state_machine_request(|sm: &mut MemStateMachine| Box::pin(async move { sm.drop_snapshot().await }))
            .await?;
    }
    let mut read = Box::pin(n0.get_snapshot());
    let polled = futures::poll!(read.as_mut());
    let pending = polled.is_pending();
    assert!(pending, "the read waits behind buffered commands");
    tracing::info!(log_index, "--- resume the worker and check exact command order");
    {
        drop(release);
        let snapshot = TypeConfig::timeout(WAIT_TIMEOUT, read).await??;
        let missing = snapshot.is_none();
        assert!(missing, "the earlier external function removed the snapshot");
        let responses = write_responses(writes).await?;
        let expected = vec![
            (log_id(1, 0, 2), ClientResponse(None)),
            (log_id(1, 0, 3), ClientResponse(Some("request-1".to_string()))),
            (log_id(1, 0, 4), ClientResponse(Some("request-2".to_string()))),
        ];
        assert_eq!(expected, responses);
    }
    tracing::info!(log_index, "--- shut down node 0");
    {
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }
    Ok(())
}

/// External state-machine functions keep their completion points while applies wait in the
/// receiver buffer, and a linearizable read issued meanwhile completes only after those applies.
///
/// `external_state_machine_request()` returns once the worker channel admits the function, before
/// the worker runs it. The function runs ahead of the applies admitted after it, and
/// `with_state_machine()` returns the result of its function.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn external_functions_with_queued_applies() -> Result<()> {
    let config = Config {
        enable_unbounded_state_machine_buffer: true,
        state_machine_channel_size: Some(2),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let mut log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- write serial 1, which the mutable function observes later"
    );
    {
        let response = router.client_request(0, "foo", 1).await?;
        log_index += 1;
        let expected_log_id = log_id(1, 0, log_index);
        assert_eq!(expected_log_id, response.log_id);
    }

    tracing::info!(log_index, "--- pause the worker");
    let release = {
        let (started, release) = admit_gate(&n0).await?;
        started.await?;
        release
    };

    tracing::info!(
        log_index,
        "--- the worker channel admits a function that clears the state machine while the worker is paused"
    );
    let mut observed = {
        let (observed_tx, mut observed_rx) = TypeConfig::oneshot();
        n0.external_state_machine_request(move |sm: &mut MemStateMachine| {
            Box::pin(async move {
                let state = sm.get_state_machine().await;
                sm.clear_state_machine().await;
                observed_tx.send(state.client_status).ok();
            })
        })
        .await?;

        let status = futures::poll!(&mut observed_rx);
        let pending = status.is_pending();
        assert!(pending, "the function has not run yet");
        observed_rx
    };

    tracing::info!(log_index, "--- commit 3 writes while the worker remains paused");
    let mut writes = vec![];
    for serial in 2..=4 {
        log_index += 1;
        let write = commit_write(&router, serial, log_index).await?;
        writes.push(write);
    }

    tracing::info!(log_index, "--- start a linearizable read while the applies wait");
    let mut barrier = Box::pin(n0.ensure_linearizable(ReadPolicy::ReadIndex));
    {
        let status = futures::poll!(barrier.as_mut());
        let pending = status.is_pending();
        assert!(pending, "the read waits for the queued applies");
    }

    tracing::info!(log_index, "--- release the worker");
    drop(release);

    tracing::info!(
        log_index,
        "--- the function runs before the queued applies, and the read completes after all of them"
    );
    {
        let observed_status = TypeConfig::timeout(WAIT_TIMEOUT, &mut observed).await??;
        let expected_observed = hashmap! {"foo".to_string() => "request-1".to_string()};
        assert_eq!(expected_observed, observed_status);

        let read = TypeConfig::timeout(WAIT_TIMEOUT, barrier).await??;
        let read_log_id = read.log_id();
        let expected_read_log_id = log_id(1, 0, log_index);
        assert_eq!(&expected_read_log_id, read_log_id);

        let state = n0
            .with_state_machine(|sm: &mut MemStateMachine| Box::pin(async move { sm.get_state_machine().await }))
            .await?;
        let expected_applied = Some(log_id(1, 0, log_index));
        assert_eq!(expected_applied, state.last_applied_log);
        let expected_status = hashmap! {"foo".to_string() => "request-4".to_string()};
        assert_eq!(expected_status, state.client_status);

        // The cleared state machine has no previous status for serial 2.
        let responses = write_responses(writes).await?;
        let expected = vec![
            (log_id(1, 0, 3), ClientResponse(None)),
            (log_id(1, 0, 4), ClientResponse(Some("request-2".to_string()))),
            (log_id(1, 0, 5), ClientResponse(Some("request-3".to_string()))),
        ];
        assert_eq!(expected, responses);
    }

    tracing::info!(log_index, "--- shut down node 0");
    {
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }

    Ok(())
}

/// Stopping the core lets the worker drain every command already admitted to its buffer.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn shutdown_drains_buffered_commands() -> Result<()> {
    let config = Config {
        enable_unbounded_state_machine_buffer: true,
        state_machine_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    tracing::info!("--- bring up a single-node cluster");
    let mut log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    tracing::info!(log_index, "--- pause the worker and buffer four writes");
    let (started, release) = admit_gate(&n0).await?;
    started.await?;
    let mut writes = vec![];
    for serial in 1..=4 {
        log_index += 1;
        let write = commit_write(&router, serial, log_index).await?;
        writes.push(write);
    }
    tracing::info!(log_index, "--- stop the core while the state machine remains paused");
    let node = TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    {
        let request = n0.external_state_machine_request(|_sm| Box::pin(async {}));
        let result = TypeConfig::timeout(WAIT_TIMEOUT, request).await?;
        assert_eq!(Err(Fatal::Stopped), result);
    }
    tracing::info!(
        log_index,
        "--- resume the worker; all admitted writes complete before channel closure"
    );
    {
        drop(release);
        let responses = write_responses(writes).await?;
        let expected = vec![
            (log_id(1, 0, 2), ClientResponse(None)),
            (log_id(1, 0, 3), ClientResponse(Some("request-1".to_string()))),
            (log_id(1, 0, 4), ClientResponse(Some("request-2".to_string()))),
            (log_id(1, 0, 5), ClientResponse(Some("request-3".to_string()))),
        ];
        assert_eq!(expected, responses);
        let state = node.state_machine.get_state_machine().await;
        let expected_applied = Some(log_id(1, 0, log_index));
        assert_eq!(expected_applied, state.last_applied_log);
        let expected_status = hashmap! {"foo".to_string() => "request-4".to_string()};
        assert_eq!(expected_status, state.client_status);
    }
    Ok(())
}

/// The receiver continues buffering while the worker is blocked on a full notification channel.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn both_queues_full_with_capacity_one() -> Result<()> {
    let config = Config {
        enable_unbounded_state_machine_buffer: true,
        state_machine_channel_size: Some(1),
        notification_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let mut log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let (_log_store, sm) = router.get_storage_handle(&0)?;

    let snapshot_log_id = log_id(1, 0, log_index);
    tracing::info!(
        log_index,
        "--- build a snapshot so that a later purge has logs to remove"
    );
    {
        n0.trigger().snapshot().await?;
        router.wait(&0).snapshot(snapshot_log_id, "snapshot built").await?;
    }

    tracing::info!(log_index, "--- pause the worker");
    let release_worker = {
        let (started, release) = admit_gate(&n0).await?;
        started.await?;
        release
    };

    tracing::info!(log_index, "--- commit 3 writes: all wait in the receiver buffer");
    let mut writes = vec![];
    for serial in 1..=3 {
        log_index += 1;
        let write = commit_write(&router, serial, log_index).await?;
        writes.push(write);
    }

    tracing::info!(log_index, "--- hold RaftCore inside a log purge");
    let purge_gate = {
        let gate = sm.block.pause(BlockOperation::PurgeLog);
        n0.trigger().purge_log(snapshot_log_id.index()).await?;
        TypeConfig::timeout(WAIT_TIMEOUT, gate.entered()).await?;
        gate
    };

    tracing::info!(
        log_index,
        "--- release the worker; its first result fills the notification slot and its second blocks"
    );
    let observed = {
        drop(release_worker);

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

    tracing::info!(log_index, "--- release RaftCore; every queue drains");
    {
        purge_gate.release();

        let observed_applied = TypeConfig::timeout(WAIT_TIMEOUT, observed).await??;
        let expected_observed = Some(log_id(1, 0, 4));
        assert_eq!(expected_observed, observed_applied);

        let responses = write_responses(writes).await?;
        let expected = vec![
            (log_id(1, 0, 2), ClientResponse(None)),
            (log_id(1, 0, 3), ClientResponse(Some("request-1".to_string()))),
            (log_id(1, 0, 4), ClientResponse(Some("request-2".to_string()))),
        ];
        assert_eq!(expected, responses);

        router.wait(&0).applied_index(Some(log_index), "every write applied").await?;
        router.wait(&0).purged(Some(snapshot_log_id), "purge completed").await?;

        let state = sm.get_state_machine().await;
        let expected_applied = Some(log_id(1, 0, log_index));
        assert_eq!(expected_applied, state.last_applied_log);
        let expected_status = hashmap! {"foo".to_string() => "request-3".to_string()};
        assert_eq!(expected_status, state.client_status);
    }

    tracing::info!(log_index, "--- shut down node 0");
    {
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }

    Ok(())
}

/// A paused state machine does not stop API processing when its bounded channel has capacity one.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn buffered_commands_do_not_block_api_input() -> Result<()> {
    let config = Config {
        enable_unbounded_state_machine_buffer: true,
        state_machine_channel_size: Some(1),
        notification_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    tracing::info!("--- bring up node 0 and pause its worker");
    let mut log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let (started, release) = admit_gate(&n0).await?;
    started.await?;
    tracing::info!(log_index, "--- admit more writes than the bounded channel can hold");
    let mut writes = vec![];
    for serial in 1..=BUFFERED_WRITES {
        log_index += 1;
        let write = commit_write(&router, serial, log_index).await?;
        writes.push(write);
    }
    tracing::info!(
        log_index,
        "--- the core continues serving state reads before applying the writes"
    );
    {
        let request = n0.with_raft_state(|state| state.local_committed().cloned());
        let observed = TypeConfig::timeout(WAIT_TIMEOUT, request).await??;
        let expected = Some(log_id(1, 0, log_index));
        assert_eq!(expected, observed);
    }
    tracing::info!(log_index, "--- resume the worker and check every response in order");
    {
        drop(release);
        let responses = write_responses(writes).await?;
        let mut expected = vec![];
        for serial in 1..=BUFFERED_WRITES {
            let previous = if serial == 1 {
                None
            } else {
                Some(format!("request-{}", serial - 1))
            };
            expected.push((log_id(1, 0, serial + 1), ClientResponse(previous)));
        }
        assert_eq!(expected, responses);
        router.wait(&0).applied_index(Some(log_index), "all buffered writes applied").await?;
    }
    tracing::info!(log_index, "--- shut down node 0");
    {
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }
    Ok(())
}

/// A leader that grants a higher vote while its applies wait in the receiver buffer steps down and
/// persists the vote through its capacity-one self-feedback path, then still applies and answers
/// each committed write once.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn higher_vote_with_queued_applies() -> Result<()> {
    let config = Config {
        enable_unbounded_state_machine_buffer: true,
        state_machine_channel_size: Some(2),
        notification_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let mut log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- pause the worker");
    let release_worker = {
        let (started, release) = admit_gate(&n0).await?;
        started.await?;
        release
    };

    tracing::info!(log_index, "--- commit 3 writes: all wait in the receiver buffer");
    let mut writes = vec![];
    for serial in 1..=3 {
        log_index += 1;
        let write = commit_write(&router, serial, log_index).await?;
        writes.push(write);
    }

    let higher_vote = Vote::new(2, 1);
    tracing::info!(
        log_index,
        "--- grant a higher vote, authorized as a leadership transfer so that the lease does not reject it"
    );
    {
        let request = VoteRequest {
            vote: higher_vote,
            last_log_id: Some(log_id(1, 0, log_index)),
            leadership_transfer_from: Some(Vote::new_committed(1, 0)),
        };
        let response = n0.vote(request).await?;
        let granted = response.is_granted_to(&higher_vote);
        assert!(granted);

        router.wait(&0).state(ServerState::Follower, "node 0 steps down").await?;

        let (mut log_store, _sm) = router.get_storage_handle(&0)?;
        let persisted_vote = log_store.read_vote().await?;
        assert_eq!(Some(higher_vote), persisted_vote);
    }

    tracing::info!(
        log_index,
        "--- release the worker; the committed writes are applied and answered once"
    );
    {
        drop(release_worker);

        let responses = write_responses(writes).await?;
        let expected = vec![
            (log_id(1, 0, 2), ClientResponse(None)),
            (log_id(1, 0, 3), ClientResponse(Some("request-1".to_string()))),
            (log_id(1, 0, 4), ClientResponse(Some("request-2".to_string()))),
        ];
        assert_eq!(expected, responses);

        router.wait(&0).applied_index(Some(log_index), "every committed write applied").await?;
    }

    tracing::info!(log_index, "--- shut down node 0");
    {
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }

    Ok(())
}

/// A snapshot installed on a learner whose applies wait in the receiver buffer runs after those
/// applies, so an older apply never overwrites the installed state.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_install_waits_behind_queued_applies() -> Result<()> {
    let config = Config {
        enable_unbounded_state_machine_buffer: true,
        state_machine_channel_size: Some(2),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up leader 0 with learner 1");
    let mut log_index = router.new_cluster([0], [1]).await?;
    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;

    tracing::info!(log_index, "--- pause the worker of node 1");
    let release_worker = {
        let (started, release) = admit_gate(&n1).await?;
        started.await?;
        release
    };

    tracing::info!(
        log_index,
        "--- replicate 3 writes; on node 1, applies wait in its receiver buffer"
    );
    for serial in 1..=3 {
        let response = router.client_request(0, "foo", serial).await?;
        log_index += 1;
        let expected_log_id = log_id(1, 0, log_index);
        assert_eq!(expected_log_id, response.log_id);

        // A heartbeat carries the new commit index to node 1, which then queues one apply.
        n0.trigger().heartbeat().await?;
        router
            .wait(&1)
            .committed_index(Some(log_index), format!("node 1 commits serial {}", serial))
            .await?;
    }

    tracing::info!(
        log_index,
        "--- isolate node 1, then write serial 4 and build a snapshot on node 0"
    );
    let snapshot = {
        router.set_unreachable(1, true);

        let response = router.client_request(0, "foo", 4).await?;
        log_index += 1;
        let expected_log_id = log_id(1, 0, log_index);
        assert_eq!(expected_log_id, response.log_id);

        n0.trigger().snapshot().await?;
        router.wait(&0).snapshot(expected_log_id, "node 0 snapshot built").await?;

        let snapshot = n0.get_snapshot().await?;
        let Some(snapshot) = snapshot else {
            panic!("node 0 has the built snapshot");
        };
        snapshot
    };

    tracing::info!(
        log_index,
        "--- install the snapshot on node 1; it queues behind buffered applies"
    );
    let mut install = Box::pin(n1.install_full_snapshot(Vote::new_committed(1, 0), snapshot));
    {
        let status = futures::poll!(install.as_mut());
        let pending = status.is_pending();
        assert!(pending, "the install waits for the paused worker");

        // Accepting the snapshot moves the last log id of node 1 to the snapshot at once.
        router.wait(&1).log_index(Some(log_index), "node 1 accepts the snapshot").await?;
    }

    tracing::info!(
        log_index,
        "--- release the worker of node 1; the snapshot installs after the queued applies"
    );
    {
        drop(release_worker);

        let response = TypeConfig::timeout(WAIT_TIMEOUT, install).await??;
        let expected_vote = Vote::new_committed(1, 0);
        assert_eq!(expected_vote, response.vote);

        let snapshot_log_id = log_id(1, 0, log_index);
        router.wait(&1).snapshot(snapshot_log_id, "node 1 installed the snapshot").await?;
        router.wait(&1).applied_index(Some(log_index), "node 1 applied up to the snapshot").await?;

        let (_log_store, sm1) = router.get_storage_handle(&1)?;
        let state = sm1.get_state_machine().await;
        let expected_applied = Some(snapshot_log_id);
        assert_eq!(expected_applied, state.last_applied_log);
        let expected_status = hashmap! {"foo".to_string() => "request-4".to_string()};
        assert_eq!(expected_status, state.client_status);
    }

    tracing::info!(log_index, "--- shut down both nodes");
    {
        for id in [1, 0] {
            TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(id)).await??;
        }
    }

    Ok(())
}

/// A worker panic drops buffered commands and releases their client responders.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn worker_exit_releases_buffered_responders() -> Result<()> {
    let config = Config {
        enable_unbounded_state_machine_buffer: true,
        state_machine_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let mut log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- pause the worker in a function that panics once released"
    );
    let release_worker = {
        let (started_tx, started_rx) = TypeConfig::oneshot();
        let (release_tx, release_rx) = TypeConfig::oneshot::<()>();
        n0.external_state_machine_request(move |_sm| {
            Box::pin(async move {
                started_tx.send(()).ok();
                release_rx.await.ok();
                panic!("injected state-machine worker exit");
            })
        })
        .await?;
        started_rx.await?;
        release_tx
    };

    tracing::info!(log_index, "--- commit 2 writes: both wait in the receiver buffer");
    let mut writes = vec![];
    for serial in 1..=2 {
        log_index += 1;
        let write = commit_write(&router, serial, log_index).await?;
        writes.push(write);
    }

    tracing::info!(
        log_index,
        "--- release the worker; it exits without taking another command"
    );
    {
        drop(release_worker);

        let fatal = RaftError::Fatal(Fatal::Stopped);

        let mut errors = vec![];
        for write in writes {
            let result = TypeConfig::timeout(WAIT_TIMEOUT, write).await??;
            let error = result.expect_err("the worker exits before applying any write");
            errors.push(error);
        }
        let expected = vec![fatal.clone(), fatal];
        assert_eq!(expected, errors);
    }

    tracing::info!(log_index, "--- shut down the stopped node 0");
    {
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }

    Ok(())
}

/// A worker's apply error reaches both pending writes while commands are buffered.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn worker_storage_error_releases_buffered_writes() -> Result<()> {
    let config = Config {
        enable_unbounded_state_machine_buffer: true,
        state_machine_channel_size: Some(2),
        notification_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let mut log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let (_log_store, sm) = router.get_storage_handle(&0)?;

    tracing::info!(log_index, "--- pause the worker before queuing the failing apply");
    let release_worker = {
        let (started, release) = admit_gate(&n0).await?;
        started.await?;
        release
    };

    tracing::info!(log_index, "--- buffer two writes while the worker remains paused");
    let (writes, discarded) = {
        log_index += 1;
        let first = commit_write(&router, 1, log_index).await?;
        let (tx, rx) = TypeConfig::oneshot();
        n0.external_state_machine_request(move |_sm| {
            Box::pin(async move {
                tx.send(()).ok();
            })
        })
        .await?;
        log_index += 1;
        let second = commit_write(&router, 2, log_index).await?;
        (vec![first, second], rx)
    };

    tracing::info!(
        log_index,
        "--- fail the first apply and report its error to both writers"
    );
    {
        let source = io::Error::other(APPLY_ERROR);
        sm.fail_next_apply(source);
        drop(release_worker);

        let result = TypeConfig::timeout(WAIT_TIMEOUT, discarded).await?;
        let cancelled = result.is_err();
        assert!(
            cancelled,
            "the worker drops the queued function instead of executing it"
        );

        let source = io::Error::other(APPLY_ERROR);
        let failed_log_id = log_id(1, 0, log_index - 1);
        let error_source = TypeConfig::err_from_error(&source);
        let storage_error = StorageError::apply(failed_log_id, error_source);
        let fatal = RaftError::Fatal(Fatal::StorageError(storage_error));
        let mut errors = vec![];
        for write in writes {
            let result = TypeConfig::timeout(WAIT_TIMEOUT, write).await??;
            let error = result.expect_err("the worker fails before applying either write");
            errors.push(error);
        }
        let expected = vec![fatal.clone(), fatal];
        assert_eq!(expected, errors);

        let state = sm.get_state_machine().await;
        let expected_applied = Some(log_id(1, 0, log_index - 2));
        assert_eq!(expected_applied, state.last_applied_log);
        let expected_status = hashmap! {};
        assert_eq!(expected_status, state.client_status);
    }

    tracing::info!(log_index, "--- shut down the failed node");
    {
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }

    Ok(())
}

/// A reported worker error survives buffer cleanup while the core is paused in storage.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn worker_storage_error_survives_buffer_cleanup() -> Result<()> {
    let config = Config {
        enable_unbounded_state_machine_buffer: true,
        state_machine_channel_size: Some(2),
        notification_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster and prepare logs for a purge");
    let mut log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let (_log_store, sm) = router.get_storage_handle(&0)?;
    let snapshot_log_id = log_id(1, 0, log_index);
    {
        n0.trigger().snapshot().await?;
        router.wait(&0).snapshot(snapshot_log_id, "snapshot built").await?;
    }

    tracing::info!(log_index, "--- pause the worker before queuing the failing apply");
    let release_worker = {
        let (started, release) = admit_gate(&n0).await?;
        started.await?;
        release
    };

    tracing::info!(log_index, "--- buffer two writes while the worker remains paused");
    let (writes, discarded) = {
        log_index += 1;
        let first = commit_write(&router, 1, log_index).await?;
        let (tx, rx) = TypeConfig::oneshot();
        n0.external_state_machine_request(move |_sm| {
            Box::pin(async move {
                tx.send(()).ok();
            })
        })
        .await?;
        log_index += 1;
        let second = commit_write(&router, 2, log_index).await?;
        (vec![first, second], rx)
    };

    tracing::info!(
        log_index,
        "--- hold RaftCore in a purge until the worker has reported its error and exited"
    );
    let purge_gate = {
        let gate = sm.block.pause(BlockOperation::PurgeLog);
        n0.trigger().purge_log(snapshot_log_id.index()).await?;
        TypeConfig::timeout(WAIT_TIMEOUT, gate.entered()).await?;

        let source = io::Error::other(APPLY_ERROR);
        sm.fail_next_apply(source);
        drop(release_worker);

        let result = TypeConfig::timeout(WAIT_TIMEOUT, discarded).await?;
        let cancelled = result.is_err();
        assert!(
            cancelled,
            "receiver teardown drops the queued function after the error notification is sent"
        );
        gate
    };

    tracing::info!(
        log_index,
        "--- resume RaftCore with the worker error ready and its buffer released"
    );
    {
        purge_gate.release();

        let source = io::Error::other(APPLY_ERROR);
        let failed_log_id = log_id(1, 0, log_index - 1);
        let error_source = TypeConfig::err_from_error(&source);
        let storage_error = StorageError::apply(failed_log_id, error_source);
        let fatal = RaftError::Fatal(Fatal::StorageError(storage_error));
        let mut errors = vec![];
        for write in writes {
            let result = TypeConfig::timeout(WAIT_TIMEOUT, write).await??;
            let error = result.expect_err("the worker fails before applying either write");
            errors.push(error);
        }
        let expected = vec![fatal.clone(), fatal];
        assert_eq!(expected, errors);

        let state = sm.get_state_machine().await;
        let expected_applied = Some(snapshot_log_id);
        assert_eq!(expected_applied, state.last_applied_log);
        let expected_status = hashmap! {};
        assert_eq!(expected_status, state.client_status);
    }

    tracing::info!(log_index, "--- shut down the failed node");
    {
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }

    Ok(())
}

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
