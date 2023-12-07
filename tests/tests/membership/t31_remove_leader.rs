use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::error::ClientWriteError;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::LogId;
use openraft::ServerState;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Change membership from {0,1} to {1,2,3}.
///
/// - Then the leader should step down after joint log is committed.
/// - Check logs on other node.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn remove_leader() -> Result<()> {
    // TODO(1): flaky with --features single-term-leader

    // Setup test dependencies.
    let config = Arc::new(
        Config {
            election_timeout_min: 800,
            election_timeout_max: 1000,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0,1}, btreeset! {2,3}).await?;

    // Submit a config change which adds two new nodes and removes the current leader.
    let orig_leader = router.leader().expect("expected the cluster to have a leader");
    assert_eq!(0, orig_leader, "expected original leader to be node 0");
    router.wait_for_log(&btreeset![0, 1], Some(log_index), timeout(), "add learner").await?;

    let node = router.get_raft_handle(&orig_leader)?;
    node.change_membership([1, 2, 3], false).await?;
    // 2 for change_membership
    log_index += 2;

    tracing::info!(log_index, "--- old leader commits 2 membership log");
    {
        router
            .wait(&orig_leader, timeout())
            .applied_index(Some(log_index), "old leader commits 2 membership log")
            .await?;
    }

    // Another node(e.g. node-1) in the old cluster may not commit the second membership change log.
    // Because to commit the 2nd log it only need a quorum of the new cluster.

    router
        .wait(&1, timeout())
        .applied_index_at_least(Some(log_index), "node in old cluster commits at least 1 membership log")
        .await?;

    tracing::info!(log_index, "--- new cluster commits 2 membership logs");
    {
        // leader commit a new log.
        log_index += 1;
        tracing::debug!("--- expect log_index:{}", log_index);

        for id in [2, 3] {
            router
                .wait(&id, timeout())
                .applied_index_at_least(
                    Some(log_index),
                    "node in new cluster finally commit at least one blank leader-initialize log",
                )
                .await?;
        }
    }

    tracing::info!(log_index, "--- check term in new cluster");
    {
        for id in [1, 2, 3] {
            router
                .wait(&id, timeout())
                .metrics(
                    |x| x.current_term >= 2,
                    "new cluster has term >= 2 because of new election",
                )
                .await?;
        }
    }

    tracing::info!(log_index, "--- check state of the old leader");
    {
        let metrics = router.wait(&0, timeout()).state(ServerState::Learner, "old leader steps down").await?;
        let cfg = &metrics.membership_config.membership();

        assert_eq!(metrics.current_term, 1);
        assert_eq!(metrics.last_log_index, Some(8));
        assert_eq!(metrics.last_applied, Some(LogId::new(CommittedLeaderId::new(1, 0), 8)));
        assert_eq!(metrics.membership_config.membership().get_joint_config().clone(), vec![
            btreeset![1, 2, 3]
        ]);
        assert_eq!(1, cfg.get_joint_config().len());
    }

    Ok(())
}

/// Change membership from {0,1} to {1,2,3}, keep node-0 as learner;
///
/// - The leader should NOT step down after joint log is committed.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn remove_leader_and_convert_to_learner() -> Result<()> {
    let config = Arc::new(
        Config {
            election_timeout_min: 800,
            election_timeout_max: 1000,
            enable_elect: false,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0,1}, btreeset! {2,3}).await?;

    let old_leader = 0;

    tracing::info!(log_index, "--- change membership and retain removed node as learner");
    {
        let node = router.get_raft_handle(&old_leader)?;
        node.change_membership([1, 2, 3], true).await?;
        log_index += 2;
    }

    tracing::info!(log_index, "--- old leader commits 2 membership log");
    {
        router
            .wait(&old_leader, timeout())
            .applied_index(Some(log_index), "old leader commits 2 membership log")
            .await?;
    }

    // Another node(e.g. node-1) in the old cluster may not commit the second membership change log.
    // Because to commit the 2nd log it only need a quorum of the new cluster.

    router
        .wait(&1, timeout())
        .applied_index_at_least(
            Some(log_index - 1),
            "node in old cluster commits at least 1 membership log",
        )
        .await?;

    tracing::info!(log_index, "--- wait 1 sec, old leader(non-voter) stays as a leader");
    {
        tokio::time::sleep(Duration::from_millis(1_000)).await;

        router
            .wait(&0, timeout())
            .state(
                ServerState::Leader,
                "old leader is not removed from membership, it is still a leader",
            )
            .await?;
    }

    Ok(())
}

/// Change membership from {0,1,2} to {2}. Access {2} at once.
///
/// It should not respond a ForwardToLeader error that pointing to the removed leader.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn remove_leader_access_new_cluster() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let orig_leader = 0;

    tracing::info!(log_index, "--- change membership 012 to 2");
    {
        let node = router.get_raft_handle(&orig_leader)?;
        node.change_membership([2], false).await?;
        // 2 change_membership logs
        log_index += 2;

        router
            .wait(&2, timeout())
            // The last_applied may not be updated on follower nodes:
            // because leader node-0 will steps down at once when the second membership log is
            // committed.
            .metrics(
                |x| x.last_log_index == Some(log_index),
                "new leader node-2 commits 2 membership log",
            )
            .await?;
    }

    tracing::info!(log_index, "--- old leader commits 2 membership log");
    {
        router
            .wait(&orig_leader, timeout())
            .applied_index(Some(log_index), "old leader commits 2 membership log")
            .await?;
    }

    let res = router.send_client_request(2, ClientRequest::make_request("foo", 1)).await;
    match res {
        Ok(_) => {
            unreachable!("expect error");
        }
        Err(cli_err) => match cli_err.api_error().unwrap() {
            ClientWriteError::ForwardToLeader(fwd) => {
                assert!(fwd.leader_id.is_none());
                assert!(fwd.leader_node.is_none());
            }
            _ => {
                unreachable!("expect ForwardToLeader");
            }
        },
    }

    tracing::info!(log_index, "--- elect node-2, handle write");
    {
        let n2 = router.get_raft_handle(&2)?;
        n2.runtime_config().elect(true);
        n2.wait(timeout()).state(ServerState::Leader, "node-2 elect itself").await?;
        log_index += 1;

        router.send_client_request(2, ClientRequest::make_request("foo", 1)).await?;
        log_index += 1;

        n2.wait(timeout())
            .applied_index(Some(log_index), "node-2 become leader and handle write request")
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(3_000))
}
