use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LogIdOptionExt;
use openraft::State;

use crate::fixtures::RaftRouter;

/// Change membership from {0,1} to {1,2,3}.
///
/// - Then the leader should step down after joint log is committed.
/// - Check logs on other node.
#[tokio::test(flavor = "multi_thread", worker_threads = 5)]
async fn step_down() -> Result<()> {
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

    let mut log_index = router.new_nodes_from_single(btreeset! {0,1}, btreeset! {}).await?;

    // Submit a config change which adds two new nodes and removes the current leader.
    let orig_leader = router.leader().await.expect("expected the cluster to have a leader");
    assert_eq!(0, orig_leader, "expected original leader to be node 0");
    router.new_raft_node(2).await;
    router.new_raft_node(3).await;
    router.change_membership(orig_leader, btreeset![1, 2, 3]).await?;
    log_index += 2;

    tracing::info!("--- old leader commits 2 membership log");
    {
        router
            .wait(&orig_leader, timeout())
            .await?
            .log(Some(log_index), "old leader commits 2 membership log")
            .await?;
    }

    // Another node(e.g. node-1) in the old cluster may not commit the second membership change log.
    // Because to commit the 2nd log it only need a quorum of the new cluster.

    router
        .wait(&1, timeout())
        .await?
        .log_at_least(log_index, "node in old cluster commits at least 1 membership log")
        .await?;

    tracing::info!("--- new cluster commits 2 membership logs");
    {
        // leader commit a new log.
        log_index += 1;

        for id in [2, 3] {
            router
                .wait(&id, timeout())
                .await?
                .log_at_least(
                    log_index,
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
        assert_eq!(metrics.last_log_index, Some(5));
        assert_eq!(metrics.last_applied.index(), Some(5));
        assert_eq!(cfg.get_configs().clone(), vec![btreeset![1, 2, 3]]);
        assert!(!cfg.is_in_joint_consensus());
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
