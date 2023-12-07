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

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RPCRequest;
use crate::fixtures::RaftRouter;

/// Client read tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - call the ensure_linearizable interface on the leader, and assert success.
/// - call the ensure_linearizable interface on the followers, and assert failure.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
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
        .ensure_linearizable(leader)
        .await
        .unwrap_or_else(|_| panic!("ensure_linearizable to succeed for cluster leader {}", leader));

    router.ensure_linearizable(1).await.expect_err("ensure_linearizable on follower node 1 to fail");
    router.ensure_linearizable(2).await.expect_err("ensure_linearizable on follower node 2 to fail");

    tracing::info!(log_index, "--- isolate node 1 then ensure_linearizable should work");

    router.set_network_error(1, true);
    router.ensure_linearizable(leader).await?;

    tracing::info!(log_index, "--- isolate node 2 then ensure_linearizable should fail");

    router.set_network_error(2, true);
    let rst = router.ensure_linearizable(leader).await;
    tracing::debug!(?rst, "ensure_linearizable with majority down");

    assert!(rst.is_err());

    Ok(())
}

/// - A leader that has not yet committed any log entries returns leader initialization log id(blank
///   log id).
/// - Return the last committed log id if the leader has committed any log entries.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
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

    tracing::info!(log_index = log_index, "--- node 1 appends blank log but can not commit");
    {
        let res = n1.wait(timeout()).applied_index_at_least(Some(log_index + 1), "blank log can not commit").await;
        assert!(res.is_err());
    }

    let blank_log_index = log_index + 1;

    tracing::info!("--- get_read_log_id returns blank log id");
    {
        let (read_log_id, applied) = n1.get_read_log_id().await?;
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

        let (read_log_id, applied) = n1.get_read_log_id().await?;
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

        let (read_log_id, _applied) = n1.get_read_log_id().await?;
        assert_eq!(
            read_log_id.index(),
            Some(last_committed),
            "read-log-id is the committed log"
        );
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(200))
}
