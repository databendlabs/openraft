use std::sync::Arc;
use std::time::Duration;

use anyerror::AnyError;
use anyhow::Result;
use maplit::btreeset;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::Config;
use openraft::LogIdOptionExt;
use openraft::RPCTypes;
use openraft::ReadPolicy;
use openraft::ServerState;

use crate::fixtures::ut_harness;
use crate::fixtures::RPCRequest;
use crate::fixtures::RaftRouter;

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
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    // This test is sensitive to network delay. Thus skip the network delay test
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

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

    tracing::info!(log_index, "--- isolate node 1 then ensure_linearizable should work");

    router.set_network_error(1, true);
    router.ensure_linearizable(leader, ReadPolicy::ReadIndex).await?;

    tracing::info!(log_index, "--- isolate node 2 then ensure_linearizable should fail");

    router.set_network_error(2, true);
    let rst = router.ensure_linearizable(leader, ReadPolicy::ReadIndex).await;
    tracing::debug!(?rst, "ensure_linearizable with majority down");

    assert!(rst.is_err());

    Ok(())
}

/// - A leader that has not yet committed any log entries returns leader initialization log id(blank
///   log id).
/// - Return the last committed log id if the leader has committed any log entries.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn get_read_log_id() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            heartbeat_interval: 100,
            election_timeout_min: 101,
            election_timeout_max: 102,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1}, btreeset! {}).await?;

    // Blocks append-entries to node 0, but let heartbeat pass.
    let block_to_n0 = |_router: &_, req, _id, target| {
        if target == 0 {
            match req {
                RPCRequest::AppendEntries(a) => {
                    // Heartbeat is not blocked.
                    if a.entries.is_empty() {
                        return Ok(());
                    }
                }
                _ => {
                    unreachable!();
                }
            }

            // Block append-entries to block commit.
            let any_err = AnyError::error("block append-entries to node 0");
            Err(RPCError::Network(NetworkError::new(&any_err)))
        } else {
            Ok(())
        }
    };

    tracing::info!("--- block append-entries to node 0");
    router.set_rpc_pre_hook(RPCTypes::AppendEntries, block_to_n0);

    // Expire current leader
    tokio::time::sleep(Duration::from_millis(200)).await;

    tracing::info!("--- let node 1 to become leader, append a blank log");
    let n1 = router.get_raft_handle(&1).unwrap();
    n1.trigger().elect().await?;

    n1.wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;

    tracing::info!(log_index = log_index, "--- node 1 appends blank log but can not commit");
    {
        let res = n1.wait(timeout()).applied_index_at_least(Some(log_index + 1), "blank log can not commit").await;
        assert!(res.is_err());
    }

    let blank_log_index = log_index + 1;

    tracing::info!("--- get_read_log_id returns blank log id");
    {
        let (read_log_id, applied) = n1.get_read_log_id(ReadPolicy::ReadIndex).await?;
        assert_eq!(
            read_log_id.index(),
            Some(blank_log_index),
            "read-log-id is the blank log"
        );
        assert_eq!(applied.index(), Some(log_index));
    }

    tracing::info!("--- stop blocking, write another log, get_read_log_id returns last log id");
    {
        router.rpc_pre_hook(RPCTypes::AppendEntries, None);

        n1.wait(timeout()).applied_index(Some(log_index + 1), "commit blank log").await?;
        log_index += 1;

        log_index += router.client_request_many(1, "foo", 1).await?;
        n1.wait(timeout()).applied_index(Some(log_index), "log applied to state-machine").await?;

        let (read_log_id, applied) = n1.get_read_log_id(ReadPolicy::ReadIndex).await?;
        assert_eq!(read_log_id.index(), Some(log_index), "read-log-id is the committed log");
        assert_eq!(applied.index(), Some(log_index));
    }

    let last_committed = log_index;

    tracing::info!(
        "--- block append again, write 1 log that wont commit, get_read_log_id returns last committed log id"
    );
    {
        router.set_rpc_pre_hook(RPCTypes::AppendEntries, block_to_n0);

        let r = router.clone();
        tokio::spawn(async move {
            // This will block for ever
            let _x = r.client_request_many(1, "foo", 1).await;
        });

        log_index += 1;
        n1.wait(timeout()).log_index(Some(log_index), "log appended, but not committed").await?;

        let (read_log_id, _applied) = n1.get_read_log_id(ReadPolicy::ReadIndex).await?;
        assert_eq!(
            read_log_id.index(),
            Some(last_committed),
            "read-log-id is the committed log"
        );
    };

    Ok(())
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn ensure_linearizable_with_read_index() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            heartbeat_interval: 100,
            election_timeout_min: 101,
            election_timeout_max: 102,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

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

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn ensure_linearizable_with_lease_read() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            heartbeat_interval: 100,
            election_timeout_min: 101,
            election_timeout_max: 102,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    // Get the ID of the leader, and assert that ensure_linearizable succeeds.
    let leader = router.leader().expect("leader not found");
    assert_eq!(leader, 0, "expected leader to be node 0, got {}", leader);

    let leader_handle = router.get_raft_handle(&leader).unwrap();

    tracing::info!("--- testing LeaseRead policy");
    {
        let rpc_count_before = router.get_rpc_count();
        let append_entries_count_before = *rpc_count_before.get(&RPCTypes::AppendEntries).unwrap_or(&0);

        router
            .ensure_linearizable(leader, ReadPolicy::LeaseRead)
            .await
            .unwrap_or_else(|_| panic!("ensure_linearizable with `LeaseRead` failed for leader {}", leader));

        // check RPC count, leader should **NOT** send heartbeat with LeaseRead policy
        let rpc_count_after = router.get_rpc_count();
        let append_entries_count_after = *rpc_count_after.get(&RPCTypes::AppendEntries).unwrap_or(&0);

        assert_eq!(
            append_entries_count_after, append_entries_count_before,
            "Lease policy should not send heartbeats: before={}, after={}",
            append_entries_count_before, append_entries_count_after
        );

        // lease time elapsed, lease read will return error.
        tokio::time::sleep(Duration::from_millis(config.election_timeout_max)).await;
        let rst = router.ensure_linearizable(leader, ReadPolicy::LeaseRead).await;
        tracing::debug!(?rst, "ensure_linearizable with LeaseRead after lease expired");

        assert!(rst.is_err());

        // lease read should ok after new a round of heartbeat.
        let old_quorum_acked = router.get_metrics(&leader)?.last_quorum_acked.unwrap().into_inner();
        leader_handle.trigger().heartbeat().await?;
        leader_handle
            .wait(timeout())
            .metrics(
                |m| {
                    let last_quorum_acked = m.last_quorum_acked;
                    last_quorum_acked.is_some() && last_quorum_acked.unwrap().into_inner() > old_quorum_acked
                },
                "leader heartbeat acked",
            )
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

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn ensure_linearizable_not_process_from_followers() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            heartbeat_interval: 100,
            election_timeout_min: 101,
            election_timeout_max: 102,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

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
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            heartbeat_interval: 100,
            election_timeout_min: 101,
            election_timeout_max: 102,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    // Blocks append-entries to node 0, but let heartbeat pass.
    let block_to_n0 = |_router: &_, req, _id, target| {
        if target == 0 {
            match req {
                RPCRequest::AppendEntries(a) => {
                    // Heartbeat is not blocked.
                    if a.entries.is_empty() {
                        return Ok(());
                    }
                }
                _ => {
                    unreachable!();
                }
            }

            // Block append-entries to block commit.
            let any_err = AnyError::error("block append-entries to node 0");
            Err(RPCError::Network(NetworkError::new(&any_err)))
        } else {
            Ok(())
        }
    };

    tracing::info!("--- block append-entries to node 0");

    // Expire current leader
    tokio::time::sleep(Duration::from_millis(200)).await;

    tracing::info!("--- let node 1 to become leader, append a blank log");
    let n1 = router.get_raft_handle(&1).unwrap();
    n1.trigger().elect().await?;

    n1.wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;

    tracing::info!("--- node 1 become leader, append logs");
    {
        n1.wait(timeout()).applied_index(Some(log_index + 1), "commit blank log").await?;
        log_index += 1;

        log_index += router.client_request_many(1, "foo", 1).await?;
        n1.wait(timeout()).applied_index(Some(log_index), "log applied to state-machine").await?;

        let (read_log_id, applied) = n1.get_read_log_id(ReadPolicy::ReadIndex).await?;
        assert_eq!(read_log_id.index(), Some(log_index), "read-log-id is the committed log");
        assert_eq!(applied.index(), Some(log_index));
    }

    tracing::info!("--- block n0, write another log, n0 unable to apply last log");
    {
        router.set_rpc_pre_hook(RPCTypes::AppendEntries, block_to_n0);
        log_index += router.client_request_many(1, "foo", 1).await?;
        n1.wait(timeout()).applied_index(Some(log_index), "log applied to state-machine").await?;

        let (read_log_id, applied) = n1.get_read_log_id(ReadPolicy::ReadIndex).await?;
        assert_eq!(read_log_id.index(), Some(log_index), "read-log-id is the committed log");
        assert_eq!(applied.index(), Some(log_index));

        let n0 = router.get_raft_handle(&0).unwrap();
        let metrics = n0.metrics().borrow().clone();
        let n0_applied = metrics.last_applied;
        let result = n0.wait_apply(read_log_id, n0_applied, Some(Duration::from_secs(1))).await?;
        assert_eq!(result, None, "n0 should wait timeout");
    }

    tracing::info!("--- stop blocking, write another log, n0 will apply last log");
    {
        router.rpc_pre_hook(RPCTypes::AppendEntries, None);
        let (read_log_id, applied) = n1.get_read_log_id(ReadPolicy::ReadIndex).await?;
        assert_eq!(read_log_id.index(), Some(log_index), "read-log-id is the committed log");
        assert_eq!(applied.index(), Some(log_index));

        let n0 = router.get_raft_handle(&0).unwrap();
        let metrics = n0.metrics().borrow().clone();
        let n0_applied = metrics.last_applied;
        let result = n0.wait_apply(read_log_id, n0_applied, None).await?;
        assert_eq!(result.index(), Some(log_index), "n0 should wait timeout");
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(200))
}
