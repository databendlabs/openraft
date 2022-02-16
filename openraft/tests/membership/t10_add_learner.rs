use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AddLearnerResponse;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftStorage;
use openraft::State;

use crate::fixtures::RaftRouter;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_learner_basic() -> Result<()> {
    //
    // - Add leader, expect NoChange
    // - Add a learner, expect raft to block until catching up.
    // - Re-add should fail.

    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let config = Arc::new(
        Config {
            replication_lag_threshold: 0,
            max_applied_log_to_keep: 2000, // prevent snapshot
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

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
        let sto1 = router.get_storage_handle(&1)?;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_learner_non_blocking() -> Result<()> {
    //
    // - Add leader, expect NoChange
    // - Add a learner, expect raft to block until catching up.
    // - Re-add should fail.

    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let config = Arc::new(
        Config {
            replication_lag_threshold: 0,
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- add new node node-1, in non blocking mode");
    {
        tracing::info!("--- write up to 100 logs");

        router.client_request_many(0, "learner_add", 100 - log_index as usize).await;
        log_index = 100;

        router.wait(&0, timeout()).await?.log(Some(log_index), "received 100 logs").await?;

        router.new_raft_node(1).await;
        let res = router.add_learner_with_blocking(0, 1, false).await?;

        assert_eq!(AddLearnerResponse { matched: None }, res);
    }

    Ok(())
}

/// add a learner, then shutdown the leader to make leader transfered,
/// check after new leader come, the learner can receive new log.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_learner_after_leader_transfered() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

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
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    let mut n_logs = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1], None, timeout, "empty").await?;
    router.wait_for_state(&btreeset![0, 1], State::Learner, timeout, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    n_logs += 1;

    router.wait_for_log(&btreeset![0, 1], Some(n_logs), timeout, "init").await?;
    router.assert_stable_cluster(Some(1), Some(1)).await;

    // Submit a config change which adds two new nodes and removes the current leader.
    let orig_leader = router.leader().await.expect("expected the cluster to have a leader");
    assert_eq!(0, orig_leader, "expected original leader to be node 0");

    // add a learner
    router.new_raft_node(2).await;
    router.add_learner(orig_leader, 2).await?;
    n_logs += 1;
    router.wait_for_log(&btreeset![0, 1], Some(n_logs), timeout, "add learner").await?;

    router.new_raft_node(3).await;
    router.new_raft_node(4).await;
    router.add_learner(orig_leader, 3).await?;
    router.add_learner(orig_leader, 4).await?;
    n_logs += 2;
    router.wait_for_log(&btreeset![0, 1], Some(n_logs), timeout, "add learner").await?;

    let node = router.get_raft_handle(&orig_leader)?;
    node.change_membership(btreeset![1, 3, 4], true, false).await?;

    // 2 for change_membership
    n_logs += 2;

    tracing::info!("--- old leader commits 2 membership log");
    {
        router
            .wait(&orig_leader, timeout)
            .await?
            .log(Some(n_logs), "old leader commits 2 membership log")
            .await?;
    }

    // Another node(e.g. node-1) in the old cluster may not commit the second membership change log.
    // Because to commit the 2nd log it only need a quorum of the new cluster.

    router
        .wait(&1, timeout)
        .await?
        .log_at_least(Some(n_logs), "node in old cluster commits at least 1 membership log")
        .await?;

    tracing::info!("--- new cluster commits 2 membership logs");
    {
        // leader commit a new log.
        n_logs += 1;

        for id in [3, 4] {
            router
                .wait(&id, timeout)
                .await?
                .log_at_least(
                    Some(n_logs),
                    "node in new cluster finally commit at least one blank leader-initialize log",
                )
                .await?;
        }
    }

    tracing::info!("--- check new cluster membership");
    {
        let sto = router.get_storage_handle(&1)?;
        let m = sto.get_membership().await?;
        let m = m.unwrap();

        assert_eq!(
            Membership::new_single_with_learners(btreeset! {1,3,4}, btreeset! {2}),
            m.membership,
            "membership should be overridden by the snapshot"
        );
    }

    tracing::info!("--- check learner in new cluster can receive new log");
    {
        let new_leader = router.leader().await.expect("expected the cluster to have a new leader");
        router.client_request_many(new_leader, "0", 1).await;
        n_logs += 1;
        router
            .wait_for_log(
                &btreeset![1, 2, 3, 4],
                Some(n_logs),
                timeout,
                "test learner has new log",
            )
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_micros(500))
}
