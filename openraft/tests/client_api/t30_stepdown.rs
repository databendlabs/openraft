use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::State;

use crate::fixtures::RaftRouter;

/// Client write tests.
///
/// What does this test do?
///
/// - create a stable 2-node cluster.
/// - starts a config change which adds two new nodes and removes the leader.
/// - the leader should commit the change to C0 & C1 with separate majorities and then stepdown after the config change
///   is committed.
#[tokio::test(flavor = "multi_thread", worker_threads = 5)]
async fn stepdown() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(
        Config {
            election_timeout_min: 800,
            election_timeout_max: 1000,
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    let mut n_logs = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1], n_logs, timeout(), "empty").await?;
    router.wait_for_state(&btreeset![0, 1], State::Learner, timeout(), "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    n_logs += 1;

    router.wait_for_log(&btreeset![0, 1], n_logs, timeout(), "init").await?;
    router.assert_stable_cluster(Some(1), Some(1)).await;

    // Submit a config change which adds two new nodes and removes the current leader.
    let orig_leader = router.leader().await.expect("expected the cluster to have a leader");
    assert_eq!(0, orig_leader, "expected original leader to be node 0");
    router.new_raft_node(2).await;
    router.new_raft_node(3).await;
    router.change_membership(orig_leader, btreeset![1, 2, 3]).await?;
    n_logs += 2;

    tracing::info!("--- old leader commits 2 membership log");
    {
        router
            .wait(&orig_leader, timeout())
            .await?
            .log(n_logs, "old leader commits 2 membership log")
            .await?;
    }

    // Another node(e.g. node-1) in the old cluster may not commit the second membership change log.
    // Because to commit the 2nd log it only need a quorum of the new cluster.

    router
        .wait(&1, timeout())
        .await?
        .log_at_least(n_logs, "node in old cluster commits at least 1 membership log")
        .await?;

    tracing::info!("--- new cluster commits 2 membership logs");
    {
        // leader commit a new log.
        n_logs += 1;

        for id in [2, 3] {
            router
                .wait(&id, timeout())
                .await?
                .log_at_least(
                    n_logs,
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
                .await?
                .metrics(
                    |x| x.current_term >= 2,
                    "new cluster has term >= 2 because of new election",
                )
                .await?;
        }
    }

    tracing::info!("--- check state of the old leader");
    {
        let metrics = router.get_metrics(&0).await?;
        let cfg = metrics.membership_config.membership;

        assert!(metrics.state != State::Leader);
        assert_eq!(metrics.current_term, 1);
        assert_eq!(metrics.last_log_index, Some(3));
        assert_eq!(metrics.last_applied, 3);
        assert_eq!(cfg.get_configs().clone(), vec![btreeset![1, 2, 3]]);
        assert!(!cfg.is_in_joint_consensus());
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
