use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Change membership from {0,1} to {1,2,3}.
///
/// - Then the leader should step down after joint log is committed.
/// - Check logs on other node.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn step_down() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            election_timeout_min: 800,
            election_timeout_max: 1000,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(btreeset! {0,1}, btreeset! {}).await?;

    // Submit a config change which adds two new nodes and removes the current leader.
    let orig_leader = router.leader().expect("expected the cluster to have a leader");
    assert_eq!(0, orig_leader, "expected original leader to be node 0");
    router.new_raft_node(2);
    router.new_raft_node(3);
    router.add_learner(0, 2).await?;
    router.add_learner(0, 3).await?;
    log_index += 2;
    router.wait_for_log(&btreeset![0, 1], Some(log_index), timeout(), "add learner").await?;

    let node = router.get_raft_handle(&orig_leader)?;
    node.change_membership(btreeset![1, 2, 3], true, false).await?;
    // 2 for change_membership
    log_index += 2;

    tracing::info!("--- old leader commits 2 membership log");
    {
        router
            .wait(&orig_leader, timeout())
            .log(Some(log_index), "old leader commits 2 membership log")
            .await?;
    }

    // Another node(e.g. node-1) in the old cluster may not commit the second membership change log.
    // Because to commit the 2nd log it only need a quorum of the new cluster.

    router
        .wait(&1, timeout())
        .log_at_least(Some(log_index), "node in old cluster commits at least 1 membership log")
        .await?;

    tracing::info!("--- new cluster commits 2 membership logs");
    {
        // leader commit a new log.
        log_index += 1;

        for id in [2, 3] {
            router
                .wait(&id, timeout())
                .log_at_least(
                    Some(log_index),
                    "node in new cluster finally commit at least one blank leader-initialize log",
                )
                .await?;
        }
    }

    tracing::info!("--- check term in new cluster");
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

    tracing::info!("--- check state of the old leader");
    {
        let metrics = router.get_metrics(&0)?;
        let cfg = &metrics.membership_config.membership;

        assert!(metrics.state != ServerState::Leader);
        assert_eq!(metrics.current_term, 1);
        assert_eq!(metrics.last_log_index, Some(8));
        assert_eq!(metrics.last_applied, Some(LogId::new(LeaderId::new(1, 0), 8)));
        assert_eq!(metrics.membership_config.get_configs().clone(), vec![btreeset![
            1, 2, 3
        ]]);
        assert!(!cfg.is_in_joint_consensus());
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(3_000))
}
