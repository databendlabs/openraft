use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreemap;
use maplit::btreeset;
use openraft::error::ChangeMembershipError;
use openraft::error::ClientWriteError;
use openraft::error::InProgress;
use openraft::storage::RaftLogReaderExt;
use openraft::ChangeMembers;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::LogId;
use openraft::Membership;
use openraft::StorageHelper;
use tokio::time::sleep;

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
            max_in_snapshot_log_to_keep: 2000, // prevent snapshot
            purge_batch_size: 1,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- re-adding leader commits a new log but does nothing");
    {
        let res = router.add_learner(0, 0).await?;
        log_index += 1;

        assert_eq!(log_index, res.log_id.index);
        router.wait(&0, timeout()).applied_index(Some(log_index), "commit re-adding leader log").await?;
    }

    tracing::info!(log_index, "--- add new node node-1");
    {
        tracing::info!(log_index, "--- write up to 1000 logs");
        {
            router.client_request_many(0, "learner_add", 1000 - log_index as usize).await?;
            log_index = 1000;

            tracing::info!(log_index, "--- write up to 1000 logs done");
            router.wait_for_log(&btreeset! {0}, Some(log_index), timeout(), "write 1000 logs to leader").await?;
        }

        router.new_raft_node(1).await;
        router.add_learner(0, 1).await?;
        log_index += 1;

        router.wait_for_log(&btreeset! {0,1}, Some(log_index), timeout(), "add learner").await?;

        tracing::info!(log_index, "--- add_learner blocks until the replication catches up");
        {
            let (mut sto1, _sm1) = router.get_storage_handle(&1)?;

            let logs = sto1.get_log_entries(..).await?;

            assert_eq!(log_index, logs[logs.len() - 1].log_id.index);
            // 0-th log
            assert_eq!(log_index + 1, logs.len() as u64);

            router.wait_for_log(&btreeset! {0,1}, Some(log_index), timeout(), "replication to learner").await?;
        }
    }

    tracing::info!(log_index, "--- re-add node-1, nothing changes");
    {
        let res = router.add_learner(0, 1).await?;
        log_index += 1;

        assert_eq!(log_index, res.log_id.index);
        router.wait(&0, timeout()).applied_index(Some(log_index), "commit re-adding node-1 log").await?;

        let metrics = router.get_raft_handle(&0)?.metrics().borrow().clone();
        let node_ids = metrics.membership_config.membership().nodes().map(|x| *x.0).collect::<Vec<_>>();
        assert_eq!(vec![0, 1], node_ids);
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

    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- add new node node-1, in non blocking mode");
    {
        tracing::info!(log_index, "--- write up to 100 logs");

        router.client_request_many(0, "learner_add", 100 - log_index as usize).await?;
        log_index = 100;

        router.wait(&0, timeout()).applied_index(Some(log_index), "received 100 logs").await?;

        router.new_raft_node(1).await;

        // Replication problem should not block adding-learner in non-blocking mode.
        router.set_network_error(1, true);

        let raft = router.get_raft_handle(&0)?;
        raft.add_learner(1, (), false).await?;

        let n = 6;
        for i in 0..=n {
            if i == n {
                unreachable!("no replication status is reported to metrics!");
            }

            let metrics = router.get_raft_handle(&0)?.metrics().borrow().clone();
            let repl = metrics.replication.as_ref().unwrap();

            // The result is Some(&None) when there is no success replication is made,
            // and is None if no replication attempt is made(no success or failure is reported to metrics).
            let n1_repl = repl.get(&1);
            if n1_repl.is_none() {
                tracing::info!("--- no replication attempt is made, sleep and retry: {}-th attempt", i);

                sleep(Duration::from_millis(500)).await;
                continue;
            }
            assert_eq!(Some(&None), n1_repl, "no replication state to the learner is reported");
            break;
        }
    }

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn add_learner_with_set_nodes() -> Result<()> {
    // Add learners and update nodes with ChangeMembers::SetNodes
    // Node updating is ensured by unit tests of ChangeMembers
    let config = Arc::new(
        Config {
            replication_lag_threshold: 0,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- set node 2 and 4");
    {
        router.new_raft_node(4).await;

        let raft = router.get_raft_handle(&0)?;
        raft.change_membership(ChangeMembers::SetNodes(btreemap! {2=>(), 4=>()}), true).await?;

        let metrics = router.get_raft_handle(&0)?.metrics().borrow().clone();
        let node_ids = metrics.membership_config.membership().nodes().map(|x| *x.0).collect::<Vec<_>>();
        assert_eq!(vec![0, 1, 2, 4], node_ids);
    }

    Ok(())
}

/// When the previous membership is not yet committed, add-learner should fail.
///
/// Because adding learner is also a change-membership operation, a new membership config log will
/// let raft consider the previous membership config log as committed, which is actually not.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn add_learner_when_previous_membership_not_committed() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let log_index = router.new_cluster(btreeset! {0}, btreeset! {1}).await?;

    tracing::info!(log_index, "--- block replication to prevent committing any log");
    {
        router.set_network_error(1, true);

        let node = router.get_raft_handle(&0)?;
        tokio::spawn(async move {
            let res = node.change_membership([0, 1], false).await;
            tracing::info!("do not expect res: {:?}", res);
            unreachable!("do not expect any res");
        });

        sleep(Duration::from_millis(500)).await;
    }

    tracing::info!(log_index, "--- add new node node-1, in non blocking mode");
    {
        let node = router.get_raft_handle(&0)?;
        let res = node.add_learner(2, (), true).await;
        tracing::debug!("res: {:?}", res);

        let err = res.unwrap_err().into_api_error().unwrap();
        assert_eq!(
            ClientWriteError::ChangeMembershipError(ChangeMembershipError::InProgress(InProgress {
                committed: Some(log_id(1, 0, 2)),
                membership_log_id: Some(log_id(1, 0, log_index + 1))
            })),
            err
        );
    }

    Ok(())
}

/// add a learner, then shutdown the leader to make leader transferred,
/// check after new leader come, the learner can receive new log.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn check_learner_after_leader_transferred() -> Result<()> {
    // TODO(1): flaky with --features single-term-leader

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
    let mut log_index = router.new_cluster(btreeset! {0,1}, btreeset! {2}).await?;

    // Submit a config change which adds two new nodes and removes the current leader.
    let orig_leader_id = router.leader().expect("expected the cluster to have a leader");
    assert_eq!(0, orig_leader_id, "expected original leader to be node 0");

    router.new_raft_node(3).await;
    router.new_raft_node(4).await;
    router.add_learner(orig_leader_id, 3).await?;
    router.add_learner(orig_leader_id, 4).await?;
    log_index += 2;
    router.wait_for_log(&btreeset![0, 1], Some(log_index), timeout(), "add learner").await?;

    let node = router.get_raft_handle(&orig_leader_id)?;
    node.change_membership([1, 3, 4], false).await?;
    log_index += 2; // 2 change_membership log

    tracing::info!(log_index, "--- old leader commits 2 membership log");
    {
        router
            .wait(&orig_leader_id, timeout())
            .applied_index(Some(log_index), "old leader commits 2 membership log")
            .await?;
    }

    tracing::info!(log_index, "--- new cluster commits 2 membership logs");
    {
        // leader commit a new log.
        log_index += 1;

        for id in [1, 3, 4] {
            router
                .wait(&id, timeout())
                .applied_index_at_least(
                    Some(log_index),
                    "node in new cluster finally commit at least one blank leader-initialize log",
                )
                .await?;
        }
    }

    tracing::info!(log_index, "--- check new cluster membership");
    {
        let (mut sto1, mut sm1) = router.get_storage_handle(&1)?;
        let m = StorageHelper::new(&mut sto1, &mut sm1).get_membership().await?;

        // new membership is applied, thus get_membership() only returns one entry.

        assert_eq!(
            &Membership::new(vec![btreeset! {1,3,4}], Some(btreeset! {2})),
            m.committed().membership(),
            "membership should be overridden by the snapshot"
        );
        assert_eq!(
            &Membership::new(vec![btreeset! {1,3,4}], Some(btreeset! {2})),
            m.effective().membership(),
            "membership should be overridden by the snapshot"
        );
    }

    tracing::info!(log_index, "--- check learner in new cluster can receive new log");
    {
        let new_leader = router.leader().expect("expected the cluster to have a new leader");
        router.client_request_many(new_leader, "0", 1).await?;
        log_index += 1;

        for i in [1, 2, 3, 4] {
            router.wait(&i, timeout()).applied_index_at_least(Some(log_index), "learner recv new log").await?;
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(3_000))
}

pub fn log_id(term: u64, node_id: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: CommittedLeaderId::new(term, node_id),
        index,
    }
}
