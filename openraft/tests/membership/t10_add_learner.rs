use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftLogReader;
use openraft::StorageHelper;

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
            purge_batch_size: 1,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- re-adding leader does nothing");
    {
        let res = router.add_learner(0, 0).await?;
        assert_eq!(Some(LogId::new(LeaderId::new(1, 0), log_index)), res.matched);
    }

    tracing::info!("--- add new node node-1");
    {
        tracing::info!("--- write up to 1000 logs");

        router.client_request_many(0, "learner_add", 1000 - log_index as usize).await?;
        log_index = 1000;

        tracing::info!("--- write up to 1000 logs done");

        router.wait_for_log(&btreeset! {0}, Some(log_index), timeout(), "write 1000 logs to leader").await?;

        router.new_raft_node(1);
        router.add_learner(0, 1).await?;
        log_index += 1;
        router.wait_for_log(&btreeset! {0,1}, Some(log_index), timeout(), "add learner").await?;

        tracing::info!("--- add_learner blocks until the replication catches up");
        {
            let mut sto1 = router.get_storage_handle(&1)?;

            let logs = sto1.get_log_entries(..).await?;

            assert_eq!(log_index, logs[logs.len() - 1].log_id.index);
            // 0-th log
            assert_eq!(log_index + 1, logs.len() as u64);

            router.wait_for_log(&btreeset! {0,1}, Some(log_index), timeout(), "replication to learner").await?;
        }
    }

    tracing::info!("--- re-add node-1, nothing changes");
    {
        let res = router.add_learner(0, 1).await?;
        assert_eq!(Some(LogId::new(LeaderId::new(1, 0), log_index)), res.matched);
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
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- add new node node-1, in non blocking mode");
    {
        tracing::info!("--- write up to 100 logs");

        router.client_request_many(0, "learner_add", 100 - log_index as usize).await?;
        log_index = 100;

        router.wait(&0, timeout()).log(Some(log_index), "received 100 logs").await?;

        router.new_raft_node(1);
        let raft = router.get_raft_handle(&0)?;
        let res = raft.add_learner(1, None, false).await?;

        assert_eq!(None, res.matched);
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
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster members: 0,1; learners: 2");
    let mut log_index = router.new_nodes_from_single(btreeset! {0,1}, btreeset! {2}).await?;

    // Submit a config change which adds two new nodes and removes the current leader.
    let orig_leader_id = router.leader().expect("expected the cluster to have a leader");
    assert_eq!(0, orig_leader_id, "expected original leader to be node 0");

    router.new_raft_node(3);
    router.new_raft_node(4);
    router.add_learner(orig_leader_id, 3).await?;
    router.add_learner(orig_leader_id, 4).await?;
    log_index += 2;
    router.wait_for_log(&btreeset![0, 1], Some(log_index), timeout(), "add learner").await?;

    let node = router.get_raft_handle(&orig_leader_id)?;
    node.change_membership(btreeset![1, 3, 4], true, false).await?;
    log_index += 2; // 2 change_membership log

    tracing::info!("--- old leader commits 2 membership log");
    {
        router
            .wait(&orig_leader_id, timeout())
            .log(Some(log_index), "old leader commits 2 membership log")
            .await?;
    }

    tracing::info!("--- new cluster commits 2 membership logs");
    {
        // leader commit a new log.
        log_index += 1;

        for id in [1, 3, 4] {
            router
                .wait(&id, timeout())
                .log_at_least(
                    Some(log_index),
                    "node in new cluster finally commit at least one blank leader-initialize log",
                )
                .await?;
        }
    }

    tracing::info!("--- check new cluster membership");
    {
        let mut sto1 = router.get_storage_handle(&1)?;
        let m = StorageHelper::new(&mut sto1).get_membership().await?;

        // new membership is applied, thus get_membership() only returns one entry.

        assert_eq!(
            Membership::new(vec![btreeset! {1,3,4}], Some(btreeset! {2})),
            m.committed.membership,
            "membership should be overridden by the snapshot"
        );
        assert_eq!(
            Membership::new(vec![btreeset! {1,3,4}], Some(btreeset! {2})),
            m.effective.membership,
            "membership should be overridden by the snapshot"
        );
    }

    tracing::info!("--- check learner in new cluster can receive new log");
    {
        let new_leader = router.leader().expect("expected the cluster to have a new leader");
        router.client_request_many(new_leader, "0", 1).await?;
        log_index += 1;

        for i in [1, 2, 3, 4] {
            router.wait(&i, timeout()).log_at_least(Some(log_index), "learner recv new log").await?;
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(3_000))
}
