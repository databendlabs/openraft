use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Instant;
use openraft::LogIdOptionExt;
use openraft::RPCTypes;
use openraft::ReadPolicy;
use openraft::ServerState;
use openraft::Vote;
use openraft::async_runtime::OneshotSender;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::LinearizableReadError;
use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use openraft::errors::RaftError;
use openraft::raft::linearizable_read::LinearizerOption;
use openraft::type_config::TypeConfigExt;
use openraft::vote::RaftLeaderId;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::RpcCall;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::expect_timeout;
use crate::fixtures::log_id;
use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::rpc_rule::RpcRule;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

/// Client read tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - call the ensure_linearizable interface on the leader, and assert success.
/// - call the ensure_linearizable interface on the followers, and assert failure.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_reads() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config.clone())?;
    // This test is sensitive to network delay. Thus skip the network delay test
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster([0, 1, 2, 3], []).await?;

    // Get the ID of the leader, and assert that ensure_linearizable succeeds.
    let leader = router.leader().expect("leader not found");
    assert_eq!(leader, 0, "expected leader to be node 0, got {}", leader);
    router
        .ensure_linearizable(leader, ReadPolicy::ReadIndex)
        .await
        .unwrap_or_else(|_| panic!("ensure_linearizable to succeed for cluster leader {}", leader));

    router
        .ensure_linearizable(1, ReadPolicy::ReadIndex)
        .await
        .expect_err("ensure_linearizable on follower node 1 to fail");
    router
        .ensure_linearizable(2, ReadPolicy::ReadIndex)
        .await
        .expect_err("ensure_linearizable on follower node 2 to fail");
    router
        .ensure_linearizable(3, ReadPolicy::ReadIndex)
        .await
        .expect_err("ensure_linearizable on follower node 3 to fail");

    tracing::info!(log_index, "--- isolate node 1 then ensure_linearizable should work");

    router.set_network_error(1, true);
    router.ensure_linearizable(leader, ReadPolicy::ReadIndex).await?;

    tracing::info!(log_index, "--- isolate node 2 then ensure_linearizable should fail");

    for node_id in 0..4 {
        let node = router.get_raft_handle(&node_id)?;
        node.runtime_config().tick(false);
        node.runtime_config().elect(false);
    }
    TypeConfig::sleep(Duration::from_millis(config.election_timeout_max)).await;

    router.set_network_error(2, true);
    let read_future = router.get_read_log_id(leader, ReadPolicy::ReadIndex);
    let result = TypeConfig::timeout(Duration::from_secs(2), read_future)
        .await
        .expect("pending read deadline should wake RaftCore without ticks");
    tracing::debug!(?result, "ensure_linearizable with majority down");

    let err = result.unwrap_err();
    let RaftError::APIError(LinearizableReadError::QuorumNotEnough(quorum_not_enough)) = err else {
        panic!("expected QuorumNotEnough, got {err:?}");
    };
    assert_eq!(btreeset! {0, 3}, quorum_not_enough.got);

    Ok(())
}

/// - A leader that has not yet committed any log entries returns leader initialization log id(blank
///   log id).
/// - Return the last committed log id if the leader has committed any log entries.
/// - The returned read log ID contains the current leadership.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn get_read_log_id() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        heartbeat_interval: 100,
        election_timeout_min: 101,
        election_timeout_max: 102,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster([0, 1], []).await?;

    tracing::info!("--- block data entries from node 1 to node 0 while allowing heartbeats");
    let rule = RpcRule::new(1, 0, RpcKind::AppendEntries);
    let rule = rule.matching(|request| {
        let RpcRequest::AppendEntries(request) = request else {
            return false;
        };
        let empty = request.entries.is_empty();
        !empty
    });
    let blocked_to_n0 = router.add_rpc_rule(rule);

    // Expire current leader
    TypeConfig::sleep(Duration::from_millis(200)).await;

    tracing::info!("--- let node 1 to become leader, append a blank log");
    let n1 = router.get_raft_handle(&1).unwrap();
    n1.trigger().elect(false).await?;

    n1.wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;

    let leadership = n1.metrics().borrow_watched().vote.leader_id().to_committed();

    tracing::info!(log_index = log_index, "--- node 1 appends blank log but cannot commit");
    {
        let res = n1
            .wait(Some(Duration::from_millis(1_000)))
            .applied_index_at_least(Some(log_index + 1), "blank log cannot commit")
            .await;
        expect_timeout(res)?;
    }

    let blank_log_index = log_index + 1;

    tracing::info!("--- get_read_log_id returns blank log id");
    {
        let (read_log_id, applied) = n1.get_read_log_id(ReadPolicy::ReadIndex).await?;
        assert_eq!(
            (&leadership, blank_log_index),
            (read_log_id.committed_leader_id(), read_log_id.index()),
            "read-log-id is the blank log from the current leader"
        );
        assert_eq!(applied.index(), Some(log_index));
    }

    tracing::info!("--- stop blocking, write another log, get_read_log_id returns last log id");
    {
        router.remove_rpc_rule(blocked_to_n0);

        n1.wait(timeout()).applied_index(Some(log_index + 1), "commit blank log").await?;

        let response = router.client_request(1, "foo", 0).await?;
        log_index = response.log_id.index();
        n1.wait(timeout()).applied_index(Some(log_index), "log applied to state-machine").await?;

        let (read_log_id, applied) = n1.get_read_log_id(ReadPolicy::ReadIndex).await?;
        assert_eq!(
            (&leadership, log_index),
            (read_log_id.committed_leader_id(), read_log_id.index()),
            "read-log-id is the current leader's committed log"
        );
        assert_eq!(applied.index(), Some(log_index));
    }

    let last_committed = log_index;

    tracing::info!(
        "--- block append again, write 1 log that wont commit, get_read_log_id returns last committed log id"
    );
    {
        let rule = RpcRule::new(1, 0, RpcKind::AppendEntries);
        let rule = rule.matching(|request| {
            let RpcRequest::AppendEntries(request) = request else {
                return false;
            };
            let empty = request.entries.is_empty();
            !empty
        });
        router.add_rpc_rule(rule);

        let r = router.clone();
        TypeConfig::spawn(async move {
            // This will block for ever
            let _x = r.client_request_many(1, "foo", 1).await;
        });

        log_index += 1;
        n1.wait(timeout()).log_index(Some(log_index), "log appended, but not committed").await?;

        let (read_log_id, _applied) = n1.get_read_log_id(ReadPolicy::ReadIndex).await?;
        assert_eq!(
            (&leadership, last_committed),
            (read_log_id.committed_leader_id(), read_log_id.index()),
            "read-log-id is the current leader's committed log"
        );
    };

    Ok(())
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn ensure_linearizable_with_read_index() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        heartbeat_interval: 100,
        election_timeout_min: 101,
        election_timeout_max: 102,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster([0, 1, 2], []).await?;

    // Get the ID of the leader
    let leader = router.leader().expect("leader not found");
    assert_eq!(leader, 0, "expected leader to be node 0, got {}", leader);

    tracing::info!("--- testing ReadIndex policy");
    {
        let rpc_count_before = router.get_rpc_count();
        let append_entries_count_before = *rpc_count_before.get(&RPCTypes::AppendEntries).unwrap_or(&0);

        router
            .ensure_linearizable(leader, ReadPolicy::ReadIndex)
            .await
            .unwrap_or_else(|_| panic!("ensure_linearizable with ReadIndex failed for leader {}", leader));

        // check RPC count, leader should send heartbeat with ReadIndex policy
        let rpc_count_after = router.get_rpc_count();
        let append_entries_count_after = *rpc_count_after.get(&RPCTypes::AppendEntries).unwrap_or(&0);

        assert!(
            append_entries_count_after > append_entries_count_before,
            "ReadIndex policy should send heartbeats: before={}, after={}",
            append_entries_count_before,
            append_entries_count_after
        );

        tracing::info!(
            log_index,
            "--- isolate node 1 then ensure_linearizable with `ReadIndex` should work"
        );

        router.set_network_error(1, true);
        router.ensure_linearizable(leader, ReadPolicy::ReadIndex).await?;

        tracing::info!(
            log_index,
            "--- isolate node 2 then ensure_linearizable with `ReadIndex` should work"
        );

        router.set_network_error(2, true);
        let rst = router.ensure_linearizable(leader, ReadPolicy::ReadIndex).await;
        tracing::debug!(?rst, "ensure_linearizable with majority down");

        assert!(rst.is_err());
    }

    Ok(())
}

/// A queued read waits for `LinearizerOption::wait_timeout` instead of the leader lease, and a
/// zero wait timeout fails the read without queueing it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn ensure_linearizable_with_wait_timeout() -> Result<()> {
    // The leader lease equals `election_timeout_max`, so both wait timeouts below are far shorter
    // than the wait a read would inherit from the lease.
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        heartbeat_interval: 100,
        election_timeout_min: 999,
        election_timeout_max: 1000,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config.clone())?;
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    router.new_cluster([0, 1, 2], []).await?;

    let leader = router.get_raft_handle(&0)?;

    tracing::info!("--- isolate both followers so no quorum acknowledgement can arrive");
    router.set_network_error(1, true);
    router.set_network_error(2, true);

    let leader_lease = Duration::from_millis(config.election_timeout_max);
    let lease_read_margin = leader_lease / 2;

    tracing::info!("--- a zero wait timeout fails at once instead of queueing the read");
    {
        let option = LinearizerOption::new(Some(Duration::ZERO), true).with_wait_timeout(Duration::ZERO);

        let start = TypeConfig::now();
        let rst = leader.get_read_linearizer(option).await;
        let elapsed = start.elapsed();

        let got = expect_quorum_not_enough(rst.unwrap_err());
        assert_eq!(btreeset! {0}, got);
        assert!(
            elapsed < lease_read_margin,
            "a zero wait timeout must not wait: {:?}",
            elapsed
        );
    }

    tracing::info!("--- a short wait timeout expires long before the leader lease would");
    {
        let wait_timeout = Duration::from_millis(100);
        let option = LinearizerOption::new(Some(Duration::ZERO), true).with_wait_timeout(wait_timeout);

        let start = TypeConfig::now();
        let rst = leader.get_read_linearizer(option).await;
        let elapsed = start.elapsed();

        let got = expect_quorum_not_enough(rst.unwrap_err());
        assert_eq!(btreeset! {0}, got);
        assert!(
            elapsed >= wait_timeout,
            "the read must wait for its timeout: {:?}",
            elapsed
        );
        assert!(
            elapsed < lease_read_margin,
            "the read must not wait for the leader lease: {:?}",
            elapsed
        );
    }

    Ok(())
}

/// Unwrap the voter set reported by a read that failed to confirm a quorum acknowledgement.
fn expect_quorum_not_enough(err: RaftError<TypeConfig, LinearizableReadError<TypeConfig>>) -> BTreeSet<u64> {
    let RaftError::APIError(api_error) = err else {
        panic!("expected an API error");
    };
    let LinearizableReadError::QuorumNotEnough(quorum_not_enough) = api_error else {
        panic!("expected QuorumNotEnough");
    };
    quorum_not_enough.got
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn ensure_linearizable_with_lease_read() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        heartbeat_interval: 1000,
        election_timeout_min: 1001,
        election_timeout_max: 1002,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config.clone())?;
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster([0, 1, 2], []).await?;

    // Get the ID of the leader, and assert that ensure_linearizable succeeds.
    let leader = router.leader().expect("leader not found");
    assert_eq!(leader, 0, "expected leader to be node 0, got {}", leader);

    // There may be some ongoing replication requests that sending the commit log ID.
    //Wait for them to finish.
    TypeConfig::sleep(Duration::from_millis(300)).await;

    let leader_handle = router.get_raft_handle(&leader).unwrap();

    tracing::info!("--- testing LeaseRead policy");
    {
        let rpc_count_before = router.get_rpc_count();
        let before = *rpc_count_before.get(&RPCTypes::AppendEntries).unwrap_or(&0);

        router
            .ensure_linearizable(leader, ReadPolicy::LeaseRead)
            .await
            .unwrap_or_else(|_| panic!("ensure_linearizable with `LeaseRead` failed for leader {}", leader));

        // check RPC count, leader should **NOT** send heartbeat with LeaseRead policy
        let rpc_count_after = router.get_rpc_count();
        let after = *rpc_count_after.get(&RPCTypes::AppendEntries).unwrap_or(&0);

        assert_eq!(
            after, before,
            "Lease policy should not send heartbeats: counts: before={}, after={}",
            before, after
        );

        // LeaseRead fails immediately after the lease expires.
        TypeConfig::sleep(Duration::from_millis(config.election_timeout_max)).await;
        let rst = leader_handle.get_read_linearizer(ReadPolicy::LeaseRead).await;
        let got = expect_quorum_not_enough(rst.unwrap_err());
        assert_eq!(btreeset! {0}, got);

        // A new heartbeat round refreshes the lease.
        let refresh_started = TypeConfig::now();
        leader_handle.trigger().heartbeat().await?;
        leader_handle
            .wait(timeout())
            .leader_with_quorum_acked(Some(refresh_started), "leader heartbeat acked")
            .await?;

        router
            .ensure_linearizable(leader, ReadPolicy::LeaseRead)
            .await
            .unwrap_or_else(|_| panic!("ensure_linearizable with `LeaseRead` failed for leader {}", leader));

        tracing::info!(
            log_index,
            "--- isolate node 1 then ensure_linearizable with `LeaseRead` should work"
        );

        router.set_network_error(1, true);
        router.ensure_linearizable(leader, ReadPolicy::LeaseRead).await?;

        tracing::info!(
            log_index,
            "--- isolate node 2 then ensure_linearizable with `LeaseRead` should work"
        );

        router.set_network_error(2, true);
        router.ensure_linearizable(leader, ReadPolicy::LeaseRead).await?;
    }

    Ok(())
}

/// A stale read waits for a periodic heartbeat when it is configured not to send one immediately.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn linearizer_waits_for_periodic_heartbeat_when_immediate_heartbeat_disabled() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        heartbeat_interval: 100,
        election_timeout_min: 1001,
        election_timeout_max: 1002,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config.clone())?;
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster([0, 1, 2], []).await?;

    let leader = router.leader().expect("leader not found");
    assert_eq!(0, leader);

    TypeConfig::sleep(Duration::from_millis(300)).await;

    let leader_handle = router.get_raft_handle(&leader).unwrap();
    let metrics = leader_handle.metrics().borrow_watched().clone();
    let expected_leader_id = metrics.vote.leader_id().to_committed();
    let expected_applied = metrics.last_applied;

    TypeConfig::sleep(Duration::from_millis(config.election_timeout_max)).await;

    let rpc_count_before = router.get_rpc_count();
    let before = *rpc_count_before.get(&RPCTypes::AppendEntries).unwrap_or(&0);

    let option = LinearizerOption::new(None, false);
    let mut read_future = Box::pin(leader_handle.get_read_linearizer(option));
    let submitted_status = futures::poll!(read_future.as_mut());
    assert!(
        submitted_status.is_pending(),
        "the read should be submitted asynchronously"
    );

    leader_handle.with_raft_state(|_| ()).await?;
    let queued_status = futures::poll!(read_future.as_mut());
    assert!(queued_status.is_pending(), "the read should remain queued");

    let rpc_count_after = router.get_rpc_count();
    let after = *rpc_count_after.get(&RPCTypes::AppendEntries).unwrap_or(&0);
    assert_eq!(before, after, "the read should not send an immediate heartbeat");

    leader_handle.runtime_config().heartbeat(true);
    let heartbeat_wait = Duration::from_millis(config.heartbeat_interval * 5);
    let heartbeat_result = TypeConfig::timeout(heartbeat_wait, read_future).await;
    leader_handle.runtime_config().heartbeat(false);

    let linearizer = heartbeat_result.expect("periodic heartbeat should complete the queued read")?;
    assert_eq!(
        (&leader, &expected_leader_id, log_index, expected_applied.as_ref()),
        (
            linearizer.node_id(),
            linearizer.read_log_id().committed_leader_id(),
            linearizer.read_log_id().index(),
            linearizer.applied()
        )
    );

    Ok(())
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn ensure_linearizable_not_process_from_followers() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        heartbeat_interval: 100,
        election_timeout_min: 101,
        election_timeout_max: 102,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    router.new_cluster([0, 1, 2], []).await?;

    // Get the ID of the leader
    let leader = router.leader().expect("leader not found");
    assert_eq!(leader, 0, "expected leader to be node 0, got {}", leader);

    // test follower nodes with different policies
    tracing::info!("--- testing followers with different policies");
    {
        // ReadIndex from follower node 1 should fail
        router
            .ensure_linearizable(1, ReadPolicy::ReadIndex)
            .await
            .expect_err("ensure_linearizable with ReadIndex on follower node 1 should fail");

        // LeaseRead from follower node 1 should fail
        router
            .ensure_linearizable(1, ReadPolicy::LeaseRead)
            .await
            .expect_err("ensure_linearizable with LeaseRead on follower node 1 should fail");
    }

    Ok(())
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn ensure_linearizable_process_from_followers() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        heartbeat_interval: 100,
        election_timeout_min: 101,
        election_timeout_max: 102,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    router.new_cluster([0, 1, 2], []).await?;

    // Get the ID of the leader
    let leader_node_id = router.leader().expect("leader not found");
    assert_eq!(
        leader_node_id, 0,
        "expected leader to be node 0, got {}",
        leader_node_id
    );
    let leader = router.get_raft_handle(&leader_node_id).unwrap();

    // Blocks append-entries to node 1, but let heartbeat pass.
    let block_to_follower_n1 = |call: RpcCall| {
        let err = || {
            // Block append-entries to block commit.
            Err(RPCError::Network(NetworkError::<TypeConfig>::from_string(
                "block append-entries to node 0",
            )))
        };

        let res = if call.to == 1 {
            match call.request {
                RpcRequest::AppendEntries(a) => {
                    // Heartbeat is not blocked.
                    if a.entries.is_empty() { Ok(()) } else { err() }
                }
                _ => {
                    unreachable!();
                }
            }
        } else {
            Ok(())
        };

        futures::future::ready(res)
    };

    let log_index;
    tracing::info!("--- block follower n1, leader write a log, n1 unable to apply last log, but n2 does");
    {
        router.set_rpc_pre_hook(RpcKind::AppendEntries, block_to_follower_n1).await;
        let response = router.client_request(leader_node_id, "foo", 0).await?;
        log_index = response.log_id.index();
        leader.wait(timeout()).applied_index(Some(log_index), "log applied to state-machine").await?;

        let linearizer = leader.get_read_linearizer(ReadPolicy::ReadIndex).await?;
        assert_eq!(
            linearizer.read_log_id().index(),
            log_index,
            "read-log-id is the committed log"
        );
        assert_eq!(linearizer.applied().index(), Some(log_index));

        let follower_n1 = router.get_raft_handle(&1).unwrap();
        let metrics = follower_n1.metrics().borrow_watched().clone();
        let follower_n1_applied = metrics.last_applied;
        assert!(
            follower_n1_applied.as_ref() < linearizer.applied(),
            "follower applied should less than leader applied"
        );
        let res = linearizer.clone().try_await_ready(&follower_n1, Some(Duration::from_secs(1))).await?;
        println!("follower n1 res: {:?}", res);
        assert!(res.is_err(), "follower n1 should not be able to apply the last log");
        assert_eq!(
            res.unwrap_err().applied().index(),
            Some(log_index - 1),
            "follower n1 applied to {}",
            log_index - 1
        );

        let follower_n2 = router.get_raft_handle(&2).unwrap();
        let state = linearizer.await_ready(&follower_n2).await?;

        assert_eq!(
            state.applied().index(),
            Some(log_index),
            "follower n2 applied should catch up leader's applied"
        );
    }

    tracing::info!("--- stop blocking, follower n1 will apply last log");
    {
        router.clear_rpc_pre_hook(RpcKind::AppendEntries).await;

        let linearizer = leader.get_read_linearizer(ReadPolicy::ReadIndex).await?;
        assert_eq!(
            linearizer.read_log_id().index(),
            log_index,
            "read-log-id is the committed log"
        );
        assert_eq!(linearizer.applied().index(), Some(log_index));

        let follower_n1 = router.get_raft_handle(&1).unwrap();
        let state = linearizer.await_ready(&follower_n1).await?;
        assert_eq!(
            state.applied().index(),
            Some(log_index),
            "follower n1 applied should catch up leader's applied"
        );
    }

    Ok(())
}

/// A linearizer wait ends once the state machine applies an entry of a newer leadership, even at an
/// index below the read log ID.
///
/// Node 0 leads again after appending entries that no other node holds, so its read log ID is a
/// no-op after that uncommitted suffix. Node 1 then leads without the suffix, and its no-op at a
/// smaller index replaces the suffix on node 0. Within one leadership, a wait still ends only when
/// the applied log ID reaches the read log ID.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn linearizer_wait_ends_when_a_newer_leader_replaces_the_read_log_id() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        enable_leader_restore: Some(false),
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let committed = log_id(1, 0, log_index);
    let suffix_len = 3;

    tracing::info!(
        log_index,
        "--- keep entries on node 0 only; node 1 still gets heartbeats, node 2 gets nothing, so it holds no lease of node 0"
    );
    {
        let rule = RpcRule::new(0, 1, RpcKind::AppendEntries).matching(|request| {
            let RpcRequest::AppendEntries(request) = request else {
                return false;
            };
            !request.entries.is_empty()
        });
        router.add_rpc_rule(rule);
        router.add_rpc_rule(RpcRule::new(0, 2, RpcKind::AppendEntries));
    }

    tracing::info!(log_index, suffix_len, "--- append a suffix that only node 0 holds");
    {
        let n0 = router.get_raft_handle(&0)?;
        for serial in 1..=suffix_len {
            n0.write(ClientRequest::make_request("suffix", serial)).await?;
        }
        router.wait(&0).log_index(Some(log_index + suffix_len), "node 0 appends the suffix").await?;
    }

    tracing::info!(
        log_index,
        "--- restart node 0 and elect it in term 2, with its no-op after the suffix"
    );
    let (n0, noop) = {
        let node = router.stop_node(0).await?;
        router.new_raft_node_with_sto(0, node.log_store, node.state_machine).await?;
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().elect(false).await?;
        router.wait(&0).vote(Vote::new_committed(2, 0), "node 0 leads term 2").await?;

        let noop = log_id(2, 0, log_index + suffix_len + 1);
        router.wait(&0).log_index(Some(noop.index()), "node 0 appends its no-op").await?;
        (n0, noop)
    };

    tracing::info!(%noop, "--- confirm term 2 with node 1 and read at node 0's uncommitted no-op");
    let linearizer = {
        let linearizer = n0.get_read_linearizer(ReadPolicy::ReadIndex).await?;
        let read_log_id = linearizer.read_log_id();
        assert_eq!(&noop, read_log_id.log_id());
        assert_eq!(
            (&0, &noop, Some(&committed), Some(&committed)),
            (
                linearizer.node_id(),
                linearizer.noop_log_id(),
                linearizer.committed(),
                linearizer.applied()
            )
        );
        linearizer
    };

    let finite_timeout = Duration::from_secs(3_600);
    tracing::info!(
        ?finite_timeout,
        "--- pause node 0's state machine, then start an unlimited and a finite wait"
    );
    let (release_apply, mut unlimited, mut finite) = {
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

        let mut unlimited = Box::pin(linearizer.clone().await_ready(&n0));
        let mut finite = Box::pin(linearizer.try_await_ready(&n0, Some(finite_timeout)));
        let polled = futures::poll!(unlimited.as_mut());
        assert!(polled.is_pending(), "node 0 has not applied the read log ID");
        let polled = futures::poll!(finite.as_mut());
        assert!(polled.is_pending(), "node 0 has not applied the read log ID");
        (release_tx, unlimited, finite)
    };

    let newer = log_id(3, 1, log_index + 1);
    tracing::info!(
        %newer,
        "--- node 1 leads term 3 without the suffix; node 0 replaces the suffix and commits node 1's no-op but cannot apply it"
    );
    {
        let n1 = router.get_raft_handle(&1)?;
        n1.trigger().elect(false).await?;
        router.wait(&1).vote(Vote::new_committed(3, 1), "node 1 leads term 3").await?;
        router.wait(&2).applied_index(Some(newer.index()), "node 2 applies node 1's no-op").await?;

        let metrics = router
            .wait(&0)
            .metrics(|m| m.local_committed == Some(newer), "node 0 commits node 1's no-op")
            .await?;
        assert_eq!(Vote::new_committed(3, 1), metrics.vote);
        assert_eq!(
            Some(newer.index()),
            metrics.last_log_index,
            "node 1's no-op replaces the suffix"
        );
        assert_eq!(Some(committed), metrics.last_applied);

        let polled = futures::poll!(unlimited.as_mut());
        assert!(polled.is_pending(), "a higher vote and commit must not end the wait");
        let polled = futures::poll!(finite.as_mut());
        assert!(polled.is_pending(), "a higher vote and commit must not end the wait");
    }

    tracing::info!(%newer, "--- node 0 applies node 1's no-op, which ends both waits without another write");
    {
        drop(release_apply);
        let unlimited_state = TypeConfig::timeout(WAIT_TIMEOUT, unlimited).await??;
        let finite_state = TypeConfig::timeout(WAIT_TIMEOUT, finite).await??;
        let finite_state = finite_state.expect("the finite wait ends before its timeout");
        for state in [unlimited_state, finite_state] {
            let read_log_id = state.read_log_id();
            assert_eq!(&noop, read_log_id.log_id());
            assert_eq!(
                (&0, &noop, Some(&committed), Some(&newer)),
                (state.node_id(), state.noop_log_id(), state.committed(), state.applied())
            );
        }
    }

    tracing::info!(
        %newer,
        "--- within term 3, a wait on node 2 stays pending below the read log ID and ends at it"
    );
    {
        let n1 = router.get_raft_handle(&1)?;
        let n2 = router.get_raft_handle(&2)?;
        let (started_tx, started_rx) = TypeConfig::oneshot();
        let (release_tx, release_rx) = TypeConfig::oneshot::<()>();
        n2.external_state_machine_request(move |_| {
            Box::pin(async move {
                started_tx.send(()).unwrap();
                let _ = release_rx.await;
            })
        })
        .await?;
        TypeConfig::timeout(WAIT_TIMEOUT, started_rx).await??;

        let response = router.client_request(1, "term-3", 1).await?;
        let written = response.log_id;
        assert_eq!(log_id(3, 1, log_index + 2), written);
        let linearizer = n1.get_read_linearizer(ReadPolicy::ReadIndex).await?;
        let read_log_id = linearizer.read_log_id();
        assert_eq!(&written, read_log_id.log_id());

        let result = linearizer.clone().try_await_ready(&n2, Some(Duration::ZERO)).await?;
        let state = result.expect_err("node 2 has not applied the write");
        assert_eq!(Some(&newer), state.applied());
        let mut ready = Box::pin(linearizer.await_ready(&n2));
        let polled = futures::poll!(ready.as_mut());
        assert!(
            polled.is_pending(),
            "an applied log ID below the read log ID must not end the wait"
        );

        drop(release_tx);
        let state = TypeConfig::timeout(WAIT_TIMEOUT, ready).await??;
        assert_eq!(
            (&2, &newer, Some(&written), Some(&written)),
            (state.node_id(), state.noop_log_id(), state.committed(), state.applied())
        );
    }

    Ok(())
}
