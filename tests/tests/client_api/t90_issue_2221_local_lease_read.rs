use std::future::Future;
use std::task::Poll;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use maplit::hashmap;
use openraft::Config;
use openraft::EntryPayload;
use openraft::Membership;
use openraft::ReadPolicy;
use openraft::ServerState;
use openraft::SnapshotPolicy;
use openraft::Vote;
use openraft::async_runtime::OneshotSender;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::Fatal;
use openraft::errors::ForwardReason;
use openraft::errors::ForwardToLeader;
use openraft::errors::LinearizableReadError;
use openraft::impls::ProgressResponder;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::linearizable_read::LinearizerOption;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::BlockOperation;
use openraft_memstore::ClientRequest;
use openraft_memstore::ClientResponse;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::OperationGate;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::log_id;
use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::ut_harness;

/// Only enabled, positive-age reads finish while RaftCore is suspended in storage.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn cached_reads_complete_while_the_core_is_parked() -> Result<()> {
    for enabled in [false, true] {
        let config = Config {
            enable_local_lease_read: Some(enabled),
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            snapshot_policy: SnapshotPolicy::Never,
            max_in_snapshot_log_to_keep: u64::MAX,
            ..Default::default()
        };
        let mut router = RaftRouter::new(config)?;
        let log_index = router.new_cluster([0], []).await?;
        let n0 = router.get_raft_handle(&0)?;
        let (_store, sm) = router.get_storage_handle(&0)?;
        let expected = log_id(1, 0, log_index);

        tracing::info!(
            enabled,
            log_index,
            "--- prepare an applied snapshot and park the core in purge"
        );
        let gate = {
            n0.trigger().snapshot().await?;
            router.wait(&0).snapshot(expected, "snapshot prepared").await?;
            let gate = sm.block.pause(BlockOperation::PurgeLog);
            n0.trigger().purge_log(log_index).await?;
            TypeConfig::timeout(WAIT_TIMEOUT, gate.entered()).await?;
            gate
        };

        tracing::info!(
            enabled,
            log_index,
            "--- poll cached and core-only reads without letting the core resume"
        );
        {
            let before = router.get_rpc_count();
            let cases = [
                (LinearizerOption::from(ReadPolicy::LeaseRead), enabled),
                (LinearizerOption::new(Some(Duration::from_nanos(1)), false), enabled),
                (LinearizerOption::from(ReadPolicy::ReadIndex), false),
            ];
            for (option, cached) in cases {
                let mut read = Box::pin(n0.get_read_linearizer(option));
                let polled = futures::poll!(read.as_mut());
                if cached {
                    let Poll::Ready(result) = polled else {
                        panic!("a cached read must finish before RaftCore resumes");
                    };
                    let linearizer = result?;
                    let boundary = linearizer.read_log_id().log_id();
                    assert_eq!(&expected, boundary);
                    let applied = linearizer.applied();
                    assert_eq!(Some(&expected), applied);
                } else {
                    let pending = polled.is_pending();
                    assert!(pending, "the read must wait for RaftCore");
                }
            }
            let after = router.get_rpc_count();
            assert_eq!(before, after);
            if enabled {
                let mut ready = Box::pin(n0.ensure_linearizable(ReadPolicy::LeaseRead));
                let polled = futures::poll!(ready.as_mut());
                let Poll::Ready(result) = polled else {
                    panic!("known applied progress must also bypass the core");
                };
                let boundary = result?;
                let actual = boundary.log_id();
                assert_eq!(&expected, actual);
            }
        }

        tracing::info!(enabled, log_index, "--- resume the core and drain fallback requests");
        {
            gate.release();
            router.wait(&0).purged(Some(expected), "purge completed").await?;
            let boundary = n0.ensure_linearizable(ReadPolicy::ReadIndex).await?;
            let actual = boundary.log_id();
            assert_eq!(&expected, actual);
            router.stop_node(0).await?;
        }
    }
    Ok(())
}

/// A cached quorum lease expires even while the core is parked, and ReadIndex refreshes it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn cached_quorum_reads_expire_and_refresh() -> Result<()> {
    let config = Config {
        enable_local_lease_read: Some(true),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        heartbeat_interval: 100,
        election_timeout_min: 999,
        election_timeout_max: 1_000,
        snapshot_policy: SnapshotPolicy::Never,
        max_in_snapshot_log_to_keep: u64::MAX,
        ..Default::default()
    };
    let leader_lease = Duration::from_millis(config.election_timeout_max);
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let (_store, sm) = router.get_storage_handle(&0)?;
    let expected = log_id(1, 0, log_index);

    tracing::info!(log_index, "--- establish a fresh quorum lease before parking the core");
    let gate = {
        n0.trigger().snapshot().await?;
        router.wait(&0).snapshot(expected, "snapshot prepared").await?;
        let _confirmed = n0.get_read_linearizer(ReadPolicy::ReadIndex).await?;
        let gate = sm.block.pause(BlockOperation::PurgeLog);
        n0.trigger().purge_log(log_index).await?;
        TypeConfig::timeout(WAIT_TIMEOUT, gate.entered()).await?;
        gate
    };

    tracing::info!(log_index, "--- serve fresh quorum reads without resuming the core");
    {
        let custom_age = leader_lease * 2;
        let options = [
            LinearizerOption::from(ReadPolicy::LeaseRead),
            LinearizerOption::new(Some(custom_age), false).with_wait_timeout(Duration::ZERO),
        ];
        for option in options {
            let mut read = Box::pin(n0.get_read_linearizer(option));
            let polled = futures::poll!(read.as_mut());
            let Poll::Ready(result) = polled else {
                panic!("a fresh quorum lease must bypass the parked core");
            };
            let linearizer = result?;
            let boundary = linearizer.read_log_id().log_id();
            assert_eq!(&expected, boundary);
        }
    }

    tracing::info!(
        log_index,
        "--- an expired lease must fall back even with a longer custom age"
    );
    {
        let expires_at = TypeConfig::now() + leader_lease;
        TypeConfig::sleep_until(expires_at).await;
        let custom_age = leader_lease * 2;
        let options = [
            LinearizerOption::from(ReadPolicy::LeaseRead),
            LinearizerOption::new(Some(custom_age), false).with_wait_timeout(Duration::ZERO),
        ];
        for option in options {
            let mut read = Box::pin(n0.get_read_linearizer(option));
            let polled = futures::poll!(read.as_mut());
            let pending = polled.is_pending();
            assert!(pending, "an expired snapshot must wait for RaftCore");
        }
    }

    tracing::info!(
        log_index,
        "--- reject the expired lease, then refresh through ReadIndex"
    );
    {
        gate.release();
        router.wait(&0).purged(Some(expected), "purge completed").await?;
        let result = n0.get_read_linearizer(ReadPolicy::LeaseRead).await;
        let error = result.expect_err("the old quorum lease has expired");
        let api_error = error.into_api_error().expect("lease expiry is an API error");
        let LinearizableReadError::QuorumNotEnough(quorum) = api_error else {
            panic!("expected an expired quorum acknowledgement");
        };
        assert_eq!(btreeset! {0}, quorum.got);
        let _refreshed = n0.get_read_linearizer(ReadPolicy::ReadIndex).await?;
        let boundary = n0.ensure_linearizable(ReadPolicy::LeaseRead).await?;
        let actual = boundary.log_id();
        assert_eq!(&expected, actual);
    }
    Ok(())
}

/// Cache misses preserve custom wait timeouts and immediate heartbeat refresh.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn cache_misses_preserve_read_options() -> Result<()> {
    let config = Config {
        enable_local_lease_read: Some(true),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        heartbeat_interval: 100,
        election_timeout_min: 10_000,
        election_timeout_max: 10_001,
        ..Default::default()
    };
    let leader_lease = Duration::from_millis(config.election_timeout_max);
    let max_age = Duration::from_millis(10);
    let wait_timeout = Duration::from_millis(20);
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- make the custom age stale while the configured lease stays valid"
    );
    {
        let _confirmed = n0.get_read_linearizer(ReadPolicy::ReadIndex).await?;
        router.set_network_error(1, true);
        router.set_network_error(2, true);
        let stale_at = TypeConfig::now() + max_age;
        TypeConfig::sleep_until(stale_at).await;
    }

    tracing::info!(
        log_index,
        "--- honor zero and short waits without replacing them with the lease"
    );
    {
        for wait in [Duration::ZERO, wait_timeout] {
            let option = LinearizerOption::new(Some(max_age), false).with_wait_timeout(wait);
            let started = TypeConfig::now();
            let result = n0.get_read_linearizer(option).await;
            let elapsed = TypeConfig::now() - started;
            let error = result.expect_err("the isolated leader cannot refresh its quorum acknowledgement");
            let api_error = error.into_api_error().expect("a quorum timeout is an API error");
            let LinearizableReadError::QuorumNotEnough(quorum) = api_error else {
                panic!("expected the requested quorum wait to expire");
            };
            assert_eq!(btreeset! {0}, quorum.got);
            let waited = elapsed >= wait;
            assert!(waited, "the read must honor its wait timeout");
            let bounded = elapsed < leader_lease / 2;
            assert!(bounded, "a cache miss must retain the short wait timeout");
        }
    }

    tracing::info!(
        log_index,
        "--- refresh a stale custom-age read through its immediate heartbeat"
    );
    {
        router.set_network_error(1, false);
        router.set_network_error(2, false);
        let option = LinearizerOption::new(Some(max_age), true).with_wait_timeout(WAIT_TIMEOUT);
        let linearizer = n0.get_read_linearizer(option).await?;
        let boundary = linearizer.read_log_id().log_id();
        let expected = log_id(1, 0, log_index);
        assert_eq!(&expected, boundary);
    }
    Ok(())
}

/// Enabling the cache does not authorize follower, learner, or uninitialized-node reads.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn cache_enabled_replicas_forward_reads() -> Result<()> {
    let config = Config {
        enable_local_lease_read: Some(true),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0, 1], [2]).await?;
    router.new_raft_node(3).await?;

    tracing::info!(log_index, "--- replicas forward both policies to the known leader");
    {
        for node_id in [1, 2, 3] {
            let node = router.get_raft_handle(&node_id)?;
            let expected = if node_id == 3 {
                ForwardToLeader::<TypeConfig>::empty()
            } else {
                ForwardToLeader::new(0, ())
            };
            for policy in [ReadPolicy::LeaseRead, ReadPolicy::ReadIndex] {
                let result = node.get_read_linearizer(policy).await;
                let error = result.expect_err("a replica cannot authorize a leader read");
                let api_error = error.into_api_error().expect("the replica redirects the read");
                let LinearizableReadError::ForwardToLeader(forward) = api_error else {
                    panic!("expected forwarding from node {node_id}");
                };
                assert_eq!(expected, forward);
            }
        }
    }
    Ok(())
}

/// A completed write is covered by the cache before delayed apply metrics reach the core.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn cached_boundary_precedes_write_reply_and_metrics() -> Result<()> {
    let config = Config {
        enable_local_lease_read: Some(true),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        notification_channel_size: Some(1),
        state_machine_channel_size: Some(1),
        snapshot_policy: SnapshotPolicy::Never,
        max_in_snapshot_log_to_keep: u64::MAX,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let (_store, sm) = router.get_storage_handle(&0)?;
    let initial = log_id(1, 0, log_index);

    tracing::info!(log_index, "--- prepare a purge and pause the state-machine worker");
    let release_worker = {
        n0.trigger().snapshot().await?;
        router.wait(&0).snapshot(initial, "snapshot prepared").await?;
        let (started_tx, started_rx) = TypeConfig::oneshot();
        let (release_tx, release_rx) = TypeConfig::oneshot::<()>();
        n0.external_state_machine_request(move |_sm| {
            Box::pin(async move {
                started_tx.send(()).expect("test is waiting for worker entry");
                let _released = release_rx.await;
            })
        })
        .await?;
        TypeConfig::timeout(WAIT_TIMEOUT, started_rx).await??;
        release_tx
    };

    tracing::info!(
        log_index,
        "--- commit a write and verify its cached read still waits for apply"
    );
    let expected = log_id(1, 0, log_index + 1);
    let completed = {
        let request = ClientRequest::make_request("cached", 1);
        let (responder, committed, completed) = ProgressResponder::new();
        n0.write(request).responder(responder).await?;
        let committed = TypeConfig::timeout(WAIT_TIMEOUT, committed).await??;
        assert_eq!(expected, committed);
        let linearizer = n0.get_read_linearizer(ReadPolicy::LeaseRead).await?;
        let boundary = linearizer.read_log_id().log_id();
        assert_eq!(&expected, boundary);
        let mut ready = Box::pin(linearizer.try_await_ready(&n0, None));
        let polled = futures::poll!(ready.as_mut());
        let pending = polled.is_pending();
        assert!(pending, "the state machine has not applied the write");
        completed
    };

    tracing::info!(log_index, "--- park RaftCore while the worker applies and replies");
    let gate = {
        let gate = sm.block.pause(BlockOperation::PurgeLog);
        n0.trigger().purge_log(log_index).await?;
        TypeConfig::timeout(WAIT_TIMEOUT, gate.entered()).await?;
        drop(release_worker);
        let response = TypeConfig::timeout(WAIT_TIMEOUT, completed).await???;
        assert_eq!(expected, response.log_id);
        let expected_data = ClientResponse(None);
        assert_eq!(expected_data, response.data);
        gate
    };

    tracing::info!(
        log_index,
        "--- read the acknowledged boundary while metrics remain stale"
    );
    {
        let metrics_rx = n0.metrics();
        let last_applied = metrics_rx.borrow_watched().last_applied;
        assert_eq!(Some(initial), last_applied);
        let mut read = Box::pin(n0.get_read_linearizer(ReadPolicy::LeaseRead));
        let polled = futures::poll!(read.as_mut());
        let Poll::Ready(result) = polled else {
            panic!("the committed boundary must be published before the write reply");
        };
        let linearizer = result?;
        let boundary = linearizer.read_log_id().log_id();
        assert_eq!(&expected, boundary);
        let state = sm.get_state_machine().await;
        let expected_status = hashmap! {"cached".to_string() => "request-1".to_string()};
        assert_eq!(expected_status, state.client_status);
        let mut ready = Box::pin(linearizer.try_await_ready(&n0, None));
        let polled = futures::poll!(ready.as_mut());
        let pending = polled.is_pending();
        assert!(
            pending,
            "known applied progress waits for the delayed core notification"
        );
        gate.release();
        let ready = TypeConfig::timeout(WAIT_TIMEOUT, ready).await??;
        let ready = ready.expect("an unlimited apply wait completes successfully");
        let applied = ready.applied();
        assert_eq!(Some(&expected), applied);
    }
    router.stop_node(0).await?;
    Ok(())
}

/// A late acknowledgement cannot reopen the cache after transfer begins.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn transfer_keeps_cached_reads_disabled_after_a_late_ack() -> Result<()> {
    let config = Config {
        enable_local_lease_read: Some(true),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        heartbeat_interval: 1_000,
        election_timeout_min: 10_000,
        election_timeout_max: 10_001,
        ..Default::default()
    };
    let leader_lease = Duration::from_millis(config.election_timeout_max);
    let mut router = RaftRouter::new(config)?;
    router.network_send_delay(1);
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;

    tracing::info!("--- establish a lease, then hold a newer heartbeat acknowledgement");
    let (gate, heartbeat_started) = {
        n0.ensure_linearizable(ReadPolicy::ReadIndex).await?;
        router.network_send_delay(0);
        router.set_network_error(1, true);
        let gate = OperationGate::default();
        let held = gate.clone();
        router
            .set_rpc_post_hook(RpcKind::AppendEntries, move |call| {
                let held = held.clone();
                async move {
                    if call.from == 0 && call.to == 2 {
                        held.block().await;
                    }
                    Ok(())
                }
            })
            .await;
        let heartbeat_started = TypeConfig::now();
        n0.trigger().heartbeat().await?;
        TypeConfig::timeout(WAIT_TIMEOUT, gate.entered()).await?;
        (gate, heartbeat_started)
    };

    tracing::info!("--- start transfer to isolated node 1 and process the held acknowledgement");
    {
        n0.trigger().transfer_leader(1).await?;
        n0.with_raft_state(|_| ()).await?;
        let metrics_rx = n0.metrics();
        let metrics = metrics_rx.borrow_watched().clone();
        let acknowledged = metrics.last_quorum_acked.expect("the seeded lease is fresh");
        let acknowledged = acknowledged.into_inner();
        let fresh = TypeConfig::now() < acknowledged + leader_lease;
        assert!(fresh, "transfer must invalidate a lease that is still fresh");
        let older = acknowledged < heartbeat_started;
        assert!(older, "the held acknowledgement must advance the quorum clock");
        gate.release();
        n0.wait(Some(WAIT_TIMEOUT))
            .leader_with_quorum_acked(Some(heartbeat_started), "late acknowledgement handled")
            .await?;
        let result = n0.get_read_linearizer(ReadPolicy::LeaseRead).await;
        let error = result.expect_err("transfer must keep the cache disabled");
        let api_error = error.into_api_error().expect("transfer returns an API error");
        let LinearizableReadError::ForwardToLeader(forward) = api_error else {
            panic!("expected leadership-transfer forwarding");
        };
        assert_eq!(Some(1), forward.leader_id);
        assert_eq!(ForwardReason::LeadershipTransfer, forward.reason);
    }

    tracing::info!("--- let the successor write before invoking another read on the old leader");
    {
        router.clear_rpc_post_hook(RpcKind::AppendEntries).await;
        router.set_network_error(1, false);
        n0.trigger().transfer_leader(1).await?;
        router.wait(&1).state(ServerState::Leader, "successor elected").await?;
        router.wait(&1).applied_index(Some(log_index + 1), "successor no-op applied").await?;
        let response = router.client_request(1, "successor", 1).await?;
        let expected_data = ClientResponse(None);
        assert_eq!(expected_data, response.data);
        router.wait(&0).state(ServerState::Follower, "old leader stepped down").await?;
        let result = n0.get_read_linearizer(ReadPolicy::LeaseRead).await;
        let error = result.expect_err("a new invocation cannot use the old snapshot");
        let api_error = error.into_api_error().expect("old leader forwards the read");
        let LinearizableReadError::ForwardToLeader(forward) = api_error else {
            panic!("expected forwarding to the successor");
        };
        assert_eq!(Some(1), forward.leader_id);
        let linearizer = n1.get_read_linearizer(ReadPolicy::LeaseRead).await?;
        let boundary = linearizer.read_log_id().log_id();
        assert_eq!(&response.log_id, boundary);
    }
    Ok(())
}

/// Rebuilding membership revokes a cached self quorum before the new configuration commits.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn membership_rebuild_replaces_cached_quorum_requirements() -> Result<()> {
    let config = Config {
        enable_local_lease_read: Some(true),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        heartbeat_interval: 1_000,
        election_timeout_min: 10_000,
        election_timeout_max: 10_001,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    router.new_raft_node(1).await?;
    router.new_raft_node(2).await?;

    tracing::info!(log_index, "--- block new voters before replacing the self quorum");
    let gate = OperationGate::default();
    let held = gate.clone();
    router
        .set_rpc_pre_hook(RpcKind::AppendEntries, move |call| {
            let held = held.clone();
            async move {
                if call.from == 0 {
                    held.block().await;
                }
                Ok(())
            }
        })
        .await;
    let singleton = Membership::new_with_defaults(vec![btreeset! {0}], []);
    let joint = Membership::new_with_defaults(vec![btreeset! {0}, btreeset! {0, 1, 2}], []);
    let mut append = Box::pin(n0.append_membership(joint.clone(), EntryPayload::Blank, []));

    tracing::info!(
        log_index,
        "--- observe the uncommitted joint membership and reject the old cached lease"
    );
    {
        let polled = futures::poll!(append.as_mut());
        let pending = polled.is_pending();
        assert!(pending);
        let effective = n0.with_raft_state(|state| state.membership_state.effective().membership().clone()).await?;
        assert_eq!(joint, effective);
        let result = n0.get_read_linearizer(ReadPolicy::LeaseRead).await;
        let error = result.expect_err("the new voters have not acknowledged this leader");
        let api_error = error.into_api_error().expect("missing quorum is an API error");
        let LinearizableReadError::QuorumNotEnough(quorum) = api_error else {
            panic!("expected the new quorum requirement to reject the cached read");
        };
        assert_eq!(btreeset! {0}, quorum.got);
    }

    tracing::info!(
        log_index,
        "--- finish the joint configuration and restore a singleton quorum"
    );
    {
        gate.release();
        let response = TypeConfig::timeout(WAIT_TIMEOUT, append).await??;
        let expected = log_id(1, 0, log_index + 1);
        assert_eq!(expected, response.log_id);
        let response = n0.append_membership(singleton, EntryPayload::Blank, []).await?;
        let expected = log_id(1, 0, log_index + 2);
        assert_eq!(expected, response.log_id);
        router.set_network_error(1, true);
        router.set_network_error(2, true);
        let option = LinearizerOption::new(Some(Duration::from_nanos(1)), false);
        let linearizer = n0.get_read_linearizer(option).await?;
        let boundary = linearizer.read_log_id().log_id();
        assert_eq!(&expected, boundary);
    }
    Ok(())
}

/// Replacing voters can revoke an otherwise fresh quorum acknowledgement.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn membership_rebuild_can_regress_the_cached_quorum_clock() -> Result<()> {
    let config = Config {
        enable_local_lease_read: Some(true),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        heartbeat_interval: 1_000,
        election_timeout_min: 10_000,
        election_timeout_max: 10_001,
        ..Default::default()
    };
    let leader_lease = Duration::from_millis(config.election_timeout_max);
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    router.new_raft_node(3).await?;
    router.new_raft_node(4).await?;

    tracing::info!(log_index, "--- seed a fresh acknowledgement from the current voters");
    let acknowledged_at = {
        let linearizer = n0.get_read_linearizer(ReadPolicy::ReadIndex).await?;
        let expected = log_id(1, 0, log_index);
        let boundary = linearizer.read_log_id().log_id();
        assert_eq!(&expected, boundary);
        let metrics_rx = n0.metrics();
        let metrics = metrics_rx.borrow_watched().clone();
        let acknowledged = metrics.last_quorum_acked.expect("the old quorum has acknowledged this leader");
        acknowledged.into_inner()
    };

    tracing::info!(
        log_index,
        "--- block the replacement voters and publish their joint configuration"
    );
    let gate = OperationGate::default();
    let held = gate.clone();
    router
        .set_rpc_pre_hook(RpcKind::AppendEntries, move |call| {
            let held = held.clone();
            async move {
                if call.from == 0 && call.to >= 3 {
                    held.block().await;
                }
                Ok(())
            }
        })
        .await;
    let joint = Membership::new_with_defaults(vec![btreeset! {0, 1, 2}, btreeset! {0, 3, 4}], []);
    let mut append = Box::pin(n0.append_membership(joint.clone(), EntryPayload::Blank, []));

    tracing::info!(
        log_index,
        "--- reject the old fresh lease after the quorum clock regresses"
    );
    {
        let polled = futures::poll!(append.as_mut());
        let pending = polled.is_pending();
        assert!(pending);
        let effective = n0.with_raft_state(|state| state.membership_state.effective().membership().clone()).await?;
        assert_eq!(joint, effective);
        let metrics = n0
            .wait(Some(WAIT_TIMEOUT))
            .metrics(
                |metrics| metrics.last_quorum_acked.is_none(),
                "new voters have no acknowledgement",
            )
            .await?;
        assert_eq!(None, metrics.last_quorum_acked);
        let fresh = TypeConfig::now() < acknowledged_at + leader_lease;
        assert!(fresh, "the old quorum timestamp must still be fresh");
        let result = n0.get_read_linearizer(ReadPolicy::LeaseRead).await;
        let error = result.expect_err("the old quorum timestamp cannot authorize this read");
        let api_error = error.into_api_error().expect("missing quorum is an API error");
        let LinearizableReadError::QuorumNotEnough(quorum) = api_error else {
            panic!("expected the replacement voters to reject the cached read");
        };
        assert_eq!(btreeset! {0, 1, 2}, quorum.got);
    }

    tracing::info!(
        log_index,
        "--- let the replacement voters commit the joint configuration"
    );
    {
        gate.release();
        let response = TypeConfig::timeout(WAIT_TIMEOUT, append).await??;
        let expected = log_id(1, 0, log_index + 1);
        assert_eq!(expected, response.log_id);
        let linearizer = n0.get_read_linearizer(ReadPolicy::LeaseRead).await?;
        let boundary = linearizer.read_log_id().log_id();
        assert_eq!(&expected, boundary);
    }
    Ok(())
}

/// Restart discards the old publication and obtains the next term's read boundary.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn restarted_node_publishes_its_new_leadership() -> Result<()> {
    let config = Config {
        enable_local_lease_read: Some(true),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- write through the original leader and stop it");
    let node = {
        let response = router.client_request(0, "restart", 1).await?;
        let expected = log_id(1, 0, log_index + 1);
        assert_eq!(expected, response.log_id);
        let boundary = n0.ensure_linearizable(ReadPolicy::LeaseRead).await?;
        let actual = boundary.log_id();
        assert_eq!(&expected, actual);
        router.stop_node(0).await?
    };

    tracing::info!(log_index, "--- reject cached reads through the stopped leader handle");
    {
        let result = n0.get_read_linearizer(ReadPolicy::LeaseRead).await;
        let error = result.expect_err("shutdown must invalidate the cached self quorum");
        let fatal = error.into_fatal().expect("the stopped core returns a fatal error");
        assert_eq!(Fatal::Stopped, fatal);
    }

    tracing::info!(log_index, "--- restart from the stores and wait for a new leader no-op");
    {
        router.new_raft_node_with_sto(0, node.log_store, node.state_machine).await?;
        router.wait(&0).state(ServerState::Leader, "restored node elects again").await?;
        router.wait(&0).applied_index(Some(log_index + 2), "new term no-op applied").await?;
        let restarted = router.get_raft_handle(&0)?;
        let linearizer = restarted.get_read_linearizer(ReadPolicy::LeaseRead).await?;
        let expected = log_id(2, 0, log_index + 2);
        let boundary = linearizer.read_log_id().log_id();
        assert_eq!(&expected, boundary);
        let response = router.client_request(0, "restart", 2).await?;
        let expected = log_id(2, 0, log_index + 3);
        assert_eq!(expected, response.log_id);
        let expected_data = ClientResponse(Some("request-1".to_string()));
        assert_eq!(expected_data, response.data);
    }
    Ok(())
}

/// A core panic invalidates a cached self quorum before subsequent reads.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn panicked_core_invalidates_cached_self_quorum() -> Result<()> {
    let config = Config {
        enable_local_lease_read: Some(true),
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
        "--- verify a cached self-quorum read completes in the caller"
    );
    {
        let _confirmed = n0.get_read_linearizer(ReadPolicy::ReadIndex).await?;
        let mut read = Box::pin(n0.get_read_linearizer(ReadPolicy::LeaseRead));
        let polled = futures::poll!(read.as_mut());
        let Poll::Ready(result) = polled else {
            panic!("a self-quorum read must use the cached snapshot");
        };
        let linearizer = result?;
        let boundary = linearizer.read_log_id().log_id();
        let expected = log_id(1, 0, log_index);
        assert_eq!(&expected, boundary);
    }

    tracing::info!(
        log_index,
        "--- wait for the core panic before invoking another cached read"
    );
    {
        let result = n0.with_raft_state::<_, ()>(|_| panic!("injected core panic")).await;
        assert_eq!(Err(Fatal::Panicked), result);
        let result = n0.get_read_linearizer(ReadPolicy::LeaseRead).await;
        let error = result.expect_err("a panicked core must invalidate its cached self quorum");
        let fatal = error.into_fatal().expect("the core panic is fatal");
        assert_eq!(Fatal::Panicked, fatal);
    }
    Ok(())
}

/// A blocked state-machine send still invalidates the cache when a replica reports a higher vote.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn higher_vote_invalidates_cache_during_a_blocked_state_machine_send() -> Result<()> {
    let config = Config {
        enable_local_lease_read: Some(true),
        enable_unbounded_state_machine_buffer: Some(false),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        state_machine_channel_size: Some(1),
        notification_channel_size: Some(1),
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    router.network_send_delay(0);
    let log_index = router.new_cluster([0], [1]).await?;
    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;

    tracing::info!(log_index, "--- pause the worker and fill its single command slot");
    let release_worker = {
        let (started_tx, started_rx) = TypeConfig::oneshot();
        let (release_tx, release_rx) = TypeConfig::oneshot::<()>();
        n0.external_state_machine_request(move |_sm| {
            Box::pin(async move {
                started_tx.send(()).expect("test waits for the worker");
                let _released = release_rx.await;
            })
        })
        .await?;
        TypeConfig::timeout(WAIT_TIMEOUT, started_rx).await??;
        n0.external_state_machine_request(|_sm| Box::pin(async {})).await?;
        release_tx
    };

    tracing::info!(
        log_index,
        "--- hold a replication request and commit a write behind the full worker channel"
    );
    let gate = OperationGate::default();
    let held = gate.clone();
    router
        .set_rpc_pre_hook(RpcKind::AppendEntries, move |call| {
            let held = held.clone();
            async move {
                if call.from == 0 && call.to == 1 {
                    held.block().await;
                }
                Ok(())
            }
        })
        .await;
    let expected = log_id(1, 0, log_index + 1);
    let completed = {
        let request = ClientRequest::make_request("reentrant", 1);
        let (responder, committed, completed) = ProgressResponder::new();
        n0.write(request).responder(responder).await?;
        let actual = TypeConfig::timeout(WAIT_TIMEOUT, committed).await??;
        assert_eq!(expected, actual);
        TypeConfig::timeout(WAIT_TIMEOUT, gate.entered()).await?;
        let mut read = Box::pin(n0.get_read_linearizer(ReadPolicy::LeaseRead));
        let polled = futures::poll!(read.as_mut());
        let Poll::Ready(result) = polled else {
            panic!("the current self-quorum snapshot must authorize a cached read");
        };
        let linearizer = result?;
        let boundary = linearizer.read_log_id().log_id();
        assert_eq!(&expected, boundary);
        let mut barrier = Box::pin(n0.with_raft_state(|_| ()));
        let polled = futures::poll!(barrier.as_mut());
        let pending = polled.is_pending();
        assert!(pending, "the core cannot process API input during the blocked send");
        completed
    };

    tracing::info!(log_index, "--- make the held replica report a higher committed vote");
    let higher = Vote::new_committed(2, 9);
    {
        let request = AppendEntriesRequest {
            vote: higher,
            prev_log_id: Some(log_id(1, 0, log_index)),
            entries: vec![],
            leader_commit: Some(log_id(1, 0, log_index)),
        };
        n1.append_entries(request).await?;
        router.wait(&1).vote(higher, "replica learned the higher vote").await?;
        gate.release();
    }

    tracing::info!(log_index, "--- observe cache invalidation before releasing the worker");
    {
        let invalidated = futures::future::poll_fn(|cx| {
            let read = n0.get_read_linearizer(ReadPolicy::LeaseRead);
            let mut read = std::pin::pin!(read);
            match read.as_mut().poll(cx) {
                Poll::Pending => Poll::Ready(()),
                Poll::Ready(Ok(_linearizer)) => {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => panic!("the suspended core unexpectedly stopped: {error}"),
            }
        });
        TypeConfig::timeout(WAIT_TIMEOUT, invalidated).await?;
    }

    tracing::info!(
        log_index,
        "--- release the worker and complete the already committed write"
    );
    {
        drop(release_worker);
        let response = TypeConfig::timeout(WAIT_TIMEOUT, completed).await???;
        assert_eq!(expected, response.log_id);
        let expected_data = ClientResponse(None);
        assert_eq!(expected_data, response.data);
        router.wait(&0).vote(higher, "higher vote persisted after the send resumed").await?;
        let result = n0.get_read_linearizer(ReadPolicy::LeaseRead).await;
        let error = result.expect_err("the demoted node must not reopen its old snapshot");
        let api_error = error.into_api_error().expect("a follower redirects reads");
        let LinearizableReadError::ForwardToLeader(forward) = api_error else {
            panic!("expected forwarding after the higher vote");
        };
        assert_eq!(ForwardReason::NotLeader, forward.reason);
    }
    Ok(())
}
