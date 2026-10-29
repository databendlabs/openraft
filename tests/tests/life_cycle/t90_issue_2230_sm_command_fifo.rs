use std::io;

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

/// A snapshot read sent directly to the worker returns the current snapshot while an older
/// snapshot build still waits in the core's state-machine command FIFO.
///
/// The read waits for worker channel room behind the FIFO head. Gate 2 stops the worker right
/// after it grants room to both, so the read enters the channel before the queued build starts
/// its send.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_read_overtakes_queued_build() -> Result<()> {
    let config = Config {
        state_machine_channel_size: Some(3),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let mut log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    let first_snapshot = log_id(1, 0, log_index);
    tracing::info!(log_index, "--- build the snapshot that the read returns later");
    {
        n0.trigger().snapshot().await?;
        router.wait(&0).snapshot(first_snapshot, "first snapshot built").await?;
    }

    tracing::info!(log_index, "--- pause the worker with gate 1");
    let release_1 = {
        let (started_1, release_1) = admit_gate(&n0).await?;
        started_1.await?;
        release_1
    };

    tracing::info!(
        log_index,
        "--- fill the worker channel with an apply, gate 2 and an apply, then queue an apply at the FIFO head"
    );
    let mut writes = vec![];
    let (started_2, release_2) = {
        log_index += 1;
        let write = commit_write(&router, 1, log_index).await?;
        writes.push(write);

        let gate_2 = admit_gate(&n0).await?;

        for serial in 2..=3 {
            log_index += 1;
            let write = commit_write(&router, serial, log_index).await?;
            writes.push(write);
        }
        gate_2
    };

    tracing::info!(log_index, "--- queue a snapshot build behind the FIFO head");
    {
        n0.trigger().snapshot().await?;
        // RaftCore handles API messages in order: once this returns, the build is queued.
        n0.with_raft_state(|_| ()).await?;
    }

    tracing::info!(
        log_index,
        "--- start a snapshot read; it waits for channel room behind the FIFO head"
    );
    let mut read = Box::pin(n0.get_snapshot());
    {
        let status = futures::poll!(read.as_mut());
        let pending = status.is_pending();
        assert!(pending, "the worker channel is full");
    }

    tracing::info!(
        log_index,
        "--- release gate 1; the worker grants room to the FIFO head and the read, then stops at gate 2"
    );
    {
        drop(release_1);
        started_2.await?;

        // The granted room is used when the read is polled again.
        let status = futures::poll!(read.as_mut());
        let pending = status.is_pending();
        assert!(pending, "the read waits for the worker to run it");
    }

    tracing::info!(log_index, "--- release gate 2; the read returns the first snapshot");
    {
        drop(release_2);
        let snapshot = TypeConfig::timeout(WAIT_TIMEOUT, read).await??;
        let Some(snapshot) = snapshot else {
            panic!("the read returns the first snapshot");
        };
        assert_eq!(Some(first_snapshot), snapshot.meta.last_log_id);
    }

    tracing::info!(
        log_index,
        "--- the queued build runs after the FIFO head apply, and every write completes"
    );
    {
        let second_snapshot = log_id(1, 0, log_index);
        router.wait(&0).snapshot(second_snapshot, "second snapshot built").await?;

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
/// core's FIFO, and a linearizable read issued meanwhile completes only after those applies.
///
/// `external_state_machine_request()` returns once the worker channel admits the function, before
/// the worker runs it. The function runs ahead of the applies admitted after it, and
/// `with_state_machine()` returns the result of its function.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn external_functions_with_queued_applies() -> Result<()> {
    let config = Config {
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

    tracing::info!(
        log_index,
        "--- commit 3 writes: one fills the worker channel, two fill the FIFO"
    );
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

/// Shutdown stops RaftCore while the application holds the worker paused, and releases the
/// state-machine commands queued in the core.
///
/// A write whose apply never reached the worker returns `Fatal::Stopped`. A write whose apply was
/// already in the worker channel completes once the application releases the worker.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn shutdown_releases_queued_commands() -> Result<()> {
    let config = Config {
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

    tracing::info!(log_index, "--- pause the worker");
    let release = {
        let (started, release) = admit_gate(&n0).await?;
        started.await?;
        release
    };

    tracing::info!(
        log_index,
        "--- commit 4 writes: two fill the worker channel, two fill the FIFO"
    );
    let mut writes = vec![];
    for serial in 1..=4 {
        log_index += 1;
        let write = commit_write(&router, serial, log_index).await?;
        writes.push(write);
    }

    tracing::info!(log_index, "--- shut down node 0 while the worker stays paused");
    let node = TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;

    tracing::info!(
        log_index,
        "--- the writes whose applies stayed in the FIFO return Fatal::Stopped"
    );
    {
        let fifo_writes = writes.split_off(2);
        let mut errors = vec![];
        for write in fifo_writes {
            let result = TypeConfig::timeout(WAIT_TIMEOUT, write).await??;
            let error = result.expect_err("an apply left in the FIFO never runs");
            errors.push(error);
        }
        let stopped = RaftError::Fatal(Fatal::Stopped);
        assert_eq!(vec![stopped.clone(), stopped], errors);
    }

    tracing::info!(
        log_index,
        "--- no sender to the worker remains, including the one held by the pending FIFO send"
    );
    {
        let request = n0.external_state_machine_request(|_sm| Box::pin(async {}));
        let res = TypeConfig::timeout(WAIT_TIMEOUT, request).await?;
        assert_eq!(Err(Fatal::Stopped), res);
    }

    tracing::info!(
        log_index,
        "--- release the worker; the applies already in its channel complete"
    );
    {
        drop(release);

        let responses = write_responses(writes).await?;
        let expected = vec![
            (log_id(1, 0, 2), ClientResponse(None)),
            (log_id(1, 0, 3), ClientResponse(Some("request-1".to_string()))),
        ];
        assert_eq!(expected, responses);

        let state = node.state_machine.get_state_machine().await;
        let expected_applied = Some(log_id(1, 0, 3));
        assert_eq!(expected_applied, state.last_applied_log);
        let expected_status = hashmap! {"foo".to_string() => "request-2".to_string()};
        assert_eq!(expected_status, state.client_status);
    }

    Ok(())
}

/// RaftCore resumes from both bounded channels being full: the worker waits for notification
/// room while the FIFO head waits for worker channel room.
///
/// A log purge holds RaftCore while the worker fills the only notification slot and blocks on its
/// next result. Once the purge returns, RaftCore must receive notifications instead of waiting
/// for the head send alone, or neither side could proceed.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn both_queues_full_with_capacity_one() -> Result<()> {
    let config = Config {
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

    tracing::info!(
        log_index,
        "--- commit 3 writes: two fill the worker channel, one waits at the FIFO head"
    );
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

        // The worker channel has room for this function only after the worker takes the second
        // apply. The function then fills the channel, beside the room granted to the FIFO head.
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

        // The function runs after the applies that were in the worker channel and before the
        // FIFO head.
        let observed_applied = TypeConfig::timeout(WAIT_TIMEOUT, observed).await??;
        let expected_observed = Some(log_id(1, 0, 3));
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

/// A full FIFO leaves API input in its channel while RaftCore keeps handling notifications, and
/// room freed by a direct snapshot read resumes delivery without any notification.
///
/// With ticking disabled and API input gated, only the freed worker channel room can wake
/// RaftCore to send the FIFO head.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn full_fifo_defers_input_until_room_frees() -> Result<()> {
    let config = Config {
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
    let (_log_store, sm) = router.get_storage_handle(&0)?;

    let snapshot_log_id = log_id(1, 0, log_index);
    tracing::info!(
        log_index,
        "--- start a snapshot build and hold it before it reads the state machine"
    );
    let build_gate = {
        let gate = sm.block.pause(BlockOperation::DelayBuildingSnapshot);
        n0.trigger().snapshot().await?;
        TypeConfig::timeout(WAIT_TIMEOUT, gate.entered()).await?;
        gate
    };

    tracing::info!(log_index, "--- pause the worker");
    let release_worker = {
        let (started, release) = admit_gate(&n0).await?;
        started.await?;
        release
    };

    tracing::info!(log_index, "--- a snapshot read fills the worker channel");
    let mut read = Box::pin(n0.get_snapshot());
    {
        let status = futures::poll!(read.as_mut());
        let pending = status.is_pending();
        assert!(pending, "the read waits for the paused worker");
    }

    tracing::info!(log_index, "--- commit serial 1; its apply fills the FIFO");
    let write_1 = {
        log_index += 1;
        commit_write(&router, 1, log_index).await?
    };

    tracing::info!(log_index, "--- send serial 2; it stays in the API channel");
    let mut write_2 = Box::pin(n0.client_write(ClientRequest::make_request("foo", 2)));
    {
        let status = futures::poll!(write_2.as_mut());
        let pending = status.is_pending();
        assert!(pending, "serial 2 waits for its response");
    }

    tracing::info!(
        log_index,
        "--- release the snapshot build; RaftCore handles its result while the FIFO is full"
    );
    {
        build_gate.release();
        let metrics = router.wait(&0).snapshot(snapshot_log_id, "snapshot built").await?;
        assert_eq!(Some(log_index), metrics.last_log_index, "serial 2 is not appended yet");
    }

    tracing::info!(
        log_index,
        "--- release the worker; the read frees room for the FIFO head without notifying RaftCore"
    );
    {
        drop(release_worker);

        let snapshot = TypeConfig::timeout(WAIT_TIMEOUT, read).await??;
        let Some(snapshot) = snapshot else {
            panic!("the read returns the built snapshot");
        };
        assert_eq!(Some(snapshot_log_id), snapshot.meta.last_log_id);

        let responses = write_responses(vec![write_1]).await?;
        let expected = vec![(log_id(1, 0, 2), ClientResponse(None))];
        assert_eq!(expected, responses);
    }

    tracing::info!(log_index, "--- with room in the FIFO, RaftCore admits serial 2");
    {
        let response = TypeConfig::timeout(WAIT_TIMEOUT, write_2).await??;
        log_index += 1;
        let expected_log_id = log_id(1, 0, log_index);
        assert_eq!(expected_log_id, response.log_id);
        let expected_response = ClientResponse(Some("request-1".to_string()));
        assert_eq!(expected_response, response.data);

        router.wait(&0).applied_index(Some(log_index), "serial 2 applied").await?;
    }

    tracing::info!(log_index, "--- shut down node 0");
    {
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }

    Ok(())
}

/// A leader that grants a higher vote while its applies wait in the FIFO steps down and persists
/// the vote through its capacity-one self-feedback path, then still applies and answers each
/// committed write once.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn higher_vote_with_queued_applies() -> Result<()> {
    let config = Config {
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

    tracing::info!(
        log_index,
        "--- commit 3 writes: two fill the worker channel, one waits at the FIFO head"
    );
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

/// A snapshot installed on a learner whose applies wait in the FIFO runs after those applies, so
/// an older apply never overwrites the installed state.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_install_waits_behind_queued_applies() -> Result<()> {
    let config = Config {
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
        "--- replicate 3 writes; on node 1, two applies fill the worker channel and one waits at the FIFO head"
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
        "--- install the snapshot on node 1; it queues behind the FIFO head"
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

/// A worker that exits while the FIFO head waits for channel room fails that send, and RaftCore
/// stops with the storage error of the unsent apply.
///
/// Every write whose apply was queued, in the worker channel or in the FIFO, returns that fatal
/// error instead of waiting forever.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn worker_exit_fails_pending_send() -> Result<()> {
    let config = Config {
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

    tracing::info!(
        log_index,
        "--- commit 2 writes: one fills the worker channel, one waits at the FIFO head"
    );
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

        let unsent = log_id(1, 0, log_index);
        let storage_error = StorageError::apply(unsent, TypeConfig::err_from_string("channel closed"));
        let fatal = RaftError::Fatal(Fatal::StorageError(storage_error));

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

/// A worker's apply error reaches both pending writes while the FIFO head waits for room.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn worker_storage_error_fails_pending_send() -> Result<()> {
    let config = Config {
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

    tracing::info!(
        log_index,
        "--- fill the worker channel and leave the second write at the FIFO head"
    );
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

/// A queued worker error wins over channel closure even when RaftCore polls the failed send first.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn queued_worker_storage_error_precedes_failed_send() -> Result<()> {
    let config = Config {
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

    tracing::info!(
        log_index,
        "--- fill the worker channel and leave the second write at the FIFO head"
    );
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
        "--- resume RaftCore with both the failed send and worker error ready"
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
