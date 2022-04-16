use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AddLearnerResponse;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftLogReader;
use openraft::RaftStorage;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn add_learner_basic() -> Result<()> {
    //
    // - Add leader, expect NoChange
    // - Add a learner, expect raft to block until catching up.
    // - Re-add should fail.

    let config = Arc::new(
        Config {
            replication_lag_threshold: 0,
            max_applied_log_to_keep: 2000, // prevent snapshot
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- re-adding leader does nothing");
    {
        let res = router.add_learner(0, 0).await?;
        assert_eq!(
            AddLearnerResponse {
                matched: Some(LogId::new(LeaderId::new(1, 0), log_index))
            },
            res
        );
    }

    tracing::info!("--- add new node node-1");
    {
        tracing::info!("--- write up to 1000 logs");

        router.client_request_many(0, "learner_add", 1000 - log_index as usize).await;
        log_index = 1000;

        tracing::info!("--- write up to 1000 logs done");

        router.wait_for_log(&btreeset! {0}, Some(log_index), timeout(), "write 1000 logs to leader").await?;

        router.new_raft_node(1).await;
        router.add_learner(0, 1).await?;
        log_index += 1;
        router.wait_for_log(&btreeset! {0,1}, Some(log_index), timeout(), "add learner").await?;

        tracing::info!("--- add_learner blocks until the replication catches up");
        let mut sto1 = router.get_storage_handle(&1)?;

        let logs = sto1.get_log_entries(..).await?;

        assert_eq!(log_index, logs[logs.len() - 1].log_id.index);
        // 0-th log
        assert_eq!(log_index + 1, logs.len() as u64);

        router.wait_for_log(&btreeset! {0,1}, Some(log_index), timeout(), "replication to learner").await?;
    }

    tracing::info!("--- re-add node-1, expect error");
    {
        let res = router.add_learner(0, 1).await?;
        assert_eq!(
            AddLearnerResponse {
                matched: Some(LogId::new(LeaderId::new(1, 0), log_index))
            },
            res
        );
    }

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn add_learner_non_blocking() -> Result<()> {
    //
    // - Add leader, expect NoChange
    // - Add a learner, expect raft to block until catching up.
    // - Re-add should fail.

    let config = Arc::new(
        Config {
            replication_lag_threshold: 0,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- add new node node-1, in non blocking mode");
    {
        tracing::info!("--- write up to 100 logs");

        router.client_request_many(0, "learner_add", 100 - log_index as usize).await;
        log_index = 100;

        router.wait(&0, timeout())?.log(Some(log_index), "received 100 logs").await?;

        router.new_raft_node(1).await;
        let raft = router.get_raft_handle(&0)?;
        let res = raft.add_learner(1, None, false).await?;

        assert_eq!(AddLearnerResponse { matched: None }, res);
    }

    Ok(())
}

/// add a learner, then shutdown the leader to make leader transferred,
/// check after new leader come, the learner can receive new log.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn check_learner_after_leader_transfered() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            election_timeout_min: 200,
            election_timeout_max: 250,
            ..Default::default()
        }
        .validate()?,
    );
    let timeout = Some(Duration::from_millis(2000));
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    let mut log_index = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1], None, timeout, "empty").await?;
    router.wait_for_state(&btreeset![0, 1], ServerState::Learner, timeout, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    log_index += 1;

    router.wait_for_log(&btreeset![0, 1], Some(log_index), timeout, "init").await?;
    router.assert_stable_cluster(Some(1), Some(1)).await;

    // Submit a config change which adds two new nodes and removes the current leader.
    let orig_leader = router.leader().expect("expected the cluster to have a leader");
    assert_eq!(0, orig_leader, "expected original leader to be node 0");

    // add a learner
    router.new_raft_node(2).await;
    router.add_learner(orig_leader, 2).await?;
    log_index += 1;
    router.wait_for_log(&btreeset![0, 1], Some(log_index), timeout, "add learner").await?;

    router.new_raft_node(3).await;
    router.new_raft_node(4).await;
    router.add_learner(orig_leader, 3).await?;
    router.add_learner(orig_leader, 4).await?;
    log_index += 2;
    router.wait_for_log(&btreeset![0, 1], Some(log_index), timeout, "add learner").await?;

    let node = router.get_raft_handle(&orig_leader)?;
    node.change_membership(btreeset![1, 3, 4], true, false).await?;

    // 2 for change_membership
    log_index += 2;

    tracing::info!("--- old leader commits 2 membership log");
    {
        router
            .wait(&orig_leader, timeout)?
            .log(Some(log_index), "old leader commits 2 membership log")
            .await?;
    }

    // Another node(e.g. node-1) in the old cluster may not commit the second membership change log.
    // Because to commit the 2nd log it only need a quorum of the new cluster.

    router
        .wait(&1, timeout)?
        .log_at_least(Some(log_index), "node in old cluster commits at least 1 membership log")
        .await?;

    tracing::info!("--- new cluster commits 2 membership logs");
    {
        // leader commit a new log.
        log_index += 1;

        for id in [3, 4] {
            router
                .wait(&id, timeout)?
                .log_at_least(
                    Some(log_index),
                    "node in new cluster finally commit at least one blank leader-initialize log",
                )
                .await?;
        }
    }

    tracing::info!("--- check new cluster membership");
    {
        let mut sto = router.get_storage_handle(&1)?;
        let m = sto.get_membership().await?;

        assert_eq!(
            Membership::new(vec![btreeset! {1,3,4}], Some(btreeset! {2})),
            m.membership,
            "membership should be overridden by the snapshot"
        );
    }

    tracing::info!("--- check learner in new cluster can receive new log");
    {
        let new_leader = router.leader().expect("expected the cluster to have a new leader");
        router.client_request_many(new_leader, "0", 1).await;
        log_index += 1;

        for i in [1, 2, 3, 4] {
            router.wait(&i, timeout)?.log_at_least(Some(log_index), "learner recv new log").await?;
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
