use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::Config;
use openraft::LogId;
use openraft::State;
use tokio::time::sleep;

#[macro_use]
mod fixtures;

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
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    let mut n_logs = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1], n_logs, None, "empty").await?;
    router.wait_for_state(&btreeset![0, 1], State::Learner, None, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    n_logs += 1;

    router.wait_for_log(&btreeset![0, 1], n_logs, None, "init").await?;
    router.assert_stable_cluster(Some(1), Some(1)).await;

    // Submit a config change which adds two new nodes and removes the current leader.
    let orig_leader = router.leader().await.expect("expected the cluster to have a leader");
    assert_eq!(0, orig_leader, "expected original leader to be node 0");
    router.new_raft_node(2).await;
    router.new_raft_node(3).await;
    router.change_membership(orig_leader, btreeset![1, 2, 3]).await?;
    n_logs += 2;

    for id in 0..4 {
        if id == orig_leader {
            router.wait_for_log(&btreeset![id], n_logs, None, "update membership: 1, 2, 3; old leader").await?;
        } else {
            // a new leader elected and propose a log
            router
                .wait_for_log(
                    &btreeset![id],
                    n_logs + 1,
                    None,
                    "update membership: 1, 2, 3; new candidate",
                )
                .await?;
        }
    }

    // leader commit a new log.
    n_logs += 1;

    // Assert on the state of the old leader.
    {
        let metrics = router
            .latest_metrics()
            .await
            .into_iter()
            .find(|node| node.id == 0)
            .expect("expected to find metrics on original leader node");
        let cfg = metrics.membership_config.membership;
        assert!(
            metrics.state != State::Leader,
            "expected old leader to have stepped down"
        );
        assert_eq!(
            metrics.current_term, 1,
            "expected old leader to still be in first term, got {}",
            metrics.current_term
        );
        assert_eq!(
            metrics.last_log_index, 3,
            "expected old leader to have last log index of 3, got {}",
            metrics.last_log_index
        );
        assert_eq!(
            metrics.last_applied, 3,
            "expected old leader to have last applied of 3, got {}",
            metrics.last_applied
        );
        assert_eq!(
            cfg.get_configs().clone(),
            vec![btreeset![1, 2, 3]],
            "expected old leader to have membership of [1, 2, 3], got {:?}",
            cfg.get_configs()
        );
        assert!(
            !cfg.is_in_joint_consensus(),
            "expected old leader to be out of joint consensus"
        );
    }

    // Assert that the current cluster is stable.
    let _ = router.remove_node(0).await;
    sleep(Duration::from_secs(5)).await; // Give time for a new leader to be elected.

    // All metrics should be identical. Just use the first one.
    let metrics = &router.latest_metrics().await[0];

    // It may take more than one round to establish a leader.
    // As leader established it commits a Blank log.
    // If the election takes only one round, the expected term/index is 2/4.
    tracing::info!("term: {}", metrics.current_term);
    tracing::info!("index: {}", metrics.last_log_index);
    assert!(metrics.current_term >= 2, "term incr when leader changes");
    router.assert_stable_cluster(Some(metrics.current_term), Some(n_logs)).await;
    router
        .assert_storage_state(metrics.current_term, n_logs, None, LogId { term: 2, index: 4 }, None)
        .await?;
    // ----------------------------------- ^^^ this is `0` instead of `4` because blank payloads from new leaders
    //                                         and config change entries are never applied to the state machine.

    Ok(())
}
