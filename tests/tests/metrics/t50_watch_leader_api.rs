use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::ServerState;
use openraft::type_config::TypeConfigExt;
use openraft::vote::RaftLeaderId;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::events::Events;
use crate::fixtures::ut_harness;

/// Test on_cluster_leader_change API with leader switch
///
/// What does this test do?
///
/// - Creates a 3-node cluster with node 0 as initial leader
/// - Watches leader change events on node 1
/// - Shuts down node 0 to force leader change
/// - Triggers election on node 2 to become new leader
/// - Verifies leader change callbacks are invoked correctly
/// - Closes the watch handle and verifies no more callbacks are invoked
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn on_cluster_leader_change_api() -> Result<()> {
    let config = Config {
        enable_elect: false,
        enable_heartbeat: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing 3-node cluster");
    let _log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!("--- create on_cluster_leader_change on node 1");
    let n1 = router.get_raft_handle(&1)?;

    // Collect (old, new) tuples: ((term, node_id, committed), (term, node_id, committed))
    let changes = Events::default();
    let changes_clone = changes.clone();

    let mut handle = n1.on_cluster_leader_change(move |old, new| {
        let old_val = old.map(|(leader_id, committed)| (leader_id.term(), *leader_id.node_id(), committed));
        let new_val = (new.0.term(), *new.0.node_id(), new.1);
        changes_clone.push((old_val, new_val));
        async {}
    });

    changes.wait(1).await?;

    tracing::info!("--- verify initial leader change event");
    {
        let got = changes.get();
        // Expected: one callback with old=None, new=(term=1, node_id=0, committed=true)
        let want = vec![(None, (1, 0, true))];
        assert_eq!(got, want);
    }

    tracing::info!("--- shutdown node 0 (current leader)");
    router.stop_node(0).await?;

    // Wait for leader lease to expire so other nodes accept new vote
    TypeConfig::sleep(Duration::from_millis(700)).await;

    tracing::info!("--- trigger election on node 2");
    let n2 = router.get_raft_handle(&2)?;
    n2.trigger().elect(false).await?;

    tracing::info!("--- wait for node 2 to become leader");
    n2.wait(Some(Duration::from_millis(2000)))
        .state(ServerState::Leader, "wait for node 2 to become leader")
        .await?;

    tracing::info!("--- wait for node 1 to see the new leader");
    n1.wait(Some(Duration::from_millis(2000)))
        .current_leader(2, "wait for node 1 to see node 2 as leader")
        .await?;

    changes.wait(2).await?;

    tracing::info!("--- verify leader change events after election");
    {
        let got = changes.get();
        // Expected:
        // 1. Initial: old=None, new=(term=1, node_id=0, committed=true)
        // 2. New leader: old=(term=1, node_id=0, committed=true), new=(term=2, node_id=2, committed=false)
        //    Note: committed=false because callback fires at leader change moment, before vote is committed
        let want = vec![(None, (1, 0, true)), (Some((1, 0, true)), (2, 2, false))];
        assert_eq!(got, want);
    }

    tracing::info!("--- close the watch handle");
    handle.close().await;

    // Wait for leader lease to expire
    TypeConfig::sleep(Duration::from_millis(700)).await;

    tracing::info!("--- trigger election on node 1 after handle closed (node 2 still running for quorum)");
    n1.trigger().elect(false).await?;

    n1.wait(Some(Duration::from_millis(2000)))
        .state(ServerState::Leader, "wait for node 1 to become leader")
        .await?;

    // Give some time for any potential callback to be invoked
    TypeConfig::sleep(Duration::from_millis(100)).await;

    tracing::info!("--- verify no new events after handle closed");
    {
        let got = changes.get();
        // Should still be only 2 events - callback not invoked after close even though leader changed
        let want = vec![(None, (1, 0, true)), (Some((1, 0, true)), (2, 2, false))];
        assert_eq!(got, want);
    }

    Ok(())
}

/// Test on_leader_change API (simplified version for this node's leadership)
///
/// What does this test do?
///
/// - Creates a 3-node cluster with node 0 as initial leader
/// - Registers on_leader_change callbacks on node 0 (the leader)
/// - Verifies `start` callback is called when node 0 becomes committed leader
/// - Triggers election on node 2 to take over leadership
/// - Verifies `stop` callback is called when node 0 loses leadership
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn on_leader_change_api() -> Result<()> {
    let config = Config {
        enable_elect: false,
        enable_heartbeat: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing 3-node cluster");
    let _log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!("--- create on_leader_change on node 0 (the leader)");
    let n0 = router.get_raft_handle(&0)?;

    // Track start and stop events
    let started = Events::default();
    let stopped = Events::default();

    let started_clone = started.clone();
    let stopped_clone = stopped.clone();

    let mut handle = n0.on_leader_change(
        move |leader_id| {
            started_clone.push((leader_id.term(), *leader_id.node_id()));
            async {}
        },
        move |old_leader_id| {
            stopped_clone.push((old_leader_id.term(), *old_leader_id.node_id()));
            async {}
        },
    );

    started.wait(1).await?;

    tracing::info!("--- verify `start` was called for node 0");
    {
        let got = started.get();
        // Node 0 became committed leader at term 1
        assert_eq!(got, vec![(1, 0)]);

        let got = stopped.get();
        // No stop yet
        assert_eq!(got, vec![]);
    }

    // Wait for leader lease to expire so other nodes accept new vote
    TypeConfig::sleep(Duration::from_millis(700)).await;

    tracing::info!("--- trigger election on node 2");
    let n2 = router.get_raft_handle(&2)?;
    n2.trigger().elect(false).await?;

    tracing::info!("--- wait for node 2 to become leader");
    n2.wait(Some(Duration::from_millis(2000)))
        .state(ServerState::Leader, "wait for node 2 to become leader")
        .await?;

    tracing::info!("--- wait for node 0 to see the new leader");
    n0.wait(Some(Duration::from_millis(2000)))
        .current_leader(2, "wait for node 0 to see node 2 as leader")
        .await?;

    stopped.wait(1).await?;

    tracing::info!("--- verify `stop` was called for node 0");
    {
        let got = started.get();
        // Still only one start event
        assert_eq!(got, vec![(1, 0)]);

        let got = stopped.get();
        // Node 0 stopped leading (term 1, node 0)
        assert_eq!(got, vec![(1, 0)]);
    }

    handle.close().await;

    Ok(())
}

/// Test that async callbacks are actually awaited, not discarded.
///
/// This test verifies that the futures returned by callbacks are polled to completion.
/// State is only updated AFTER an await point inside the async block, so if the future
/// is discarded without being awaited, the state won't change.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn on_leader_change_future_is_awaited() -> Result<()> {
    let config = Config {
        enable_elect: false,
        enable_heartbeat: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing 3-node cluster");
    let _log_index = router.new_cluster([0, 1, 2], []).await?;

    let n0 = router.get_raft_handle(&0)?;

    let started = Events::default();
    let stopped = Events::default();

    let started_clone = started.clone();
    let stopped_clone = stopped.clone();

    tracing::info!("--- register on_leader_change with async callbacks");
    let mut handle = n0.on_leader_change(
        move |_leader_id| {
            let events = started_clone.clone();
            async move {
                // Yield to ensure this is a real async operation
                TypeConfig::yield_now().await;
                events.push(());
            }
        },
        move |_old_leader_id| {
            let events = stopped_clone.clone();
            async move {
                TypeConfig::yield_now().await;
                events.push(());
            }
        },
    );

    started.wait(1).await?;

    tracing::info!("--- verify start callback future was awaited");
    let got = started.get();
    assert_eq!(got, vec![()], "start future should have been awaited");
    let got = stopped.get();
    assert_eq!(got, vec![], "stop should not have been called yet");

    // Wait for leader lease to expire
    TypeConfig::sleep(Duration::from_millis(700)).await;

    tracing::info!("--- trigger election on node 2 to cause leadership change");
    let n2 = router.get_raft_handle(&2)?;
    n2.trigger().elect(false).await?;

    n2.wait(Some(Duration::from_millis(2000)))
        .state(ServerState::Leader, "wait for node 2 to become leader")
        .await?;

    n0.wait(Some(Duration::from_millis(2000)))
        .current_leader(2, "wait for node 0 to see node 2 as leader")
        .await?;

    stopped.wait(1).await?;

    tracing::info!("--- verify stop callback future was awaited");
    let got = started.get();
    assert_eq!(got, vec![()], "start count should still be 1");
    let got = stopped.get();
    assert_eq!(got, vec![()], "stop future should have been awaited");

    handle.close().await;

    Ok(())
}

/// Test that on_cluster_leader_change callback future is awaited.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn on_cluster_leader_change_future_is_awaited() -> Result<()> {
    let config = Config {
        enable_elect: false,
        enable_heartbeat: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing 3-node cluster");
    let _log_index = router.new_cluster([0, 1, 2], []).await?;

    let n1 = router.get_raft_handle(&1)?;

    let callbacks = Events::default();
    let callbacks_clone = callbacks.clone();

    tracing::info!("--- register on_cluster_leader_change with async callback");
    let mut handle = n1.on_cluster_leader_change(move |_old, _new| {
        let events = callbacks_clone.clone();
        async move {
            // Yield to ensure this is a real async operation
            TypeConfig::yield_now().await;
            events.push(());
        }
    });

    callbacks.wait(1).await?;

    tracing::info!("--- verify callback future was awaited for initial leader");
    let got = callbacks.get();
    assert_eq!(got, vec![()], "callback future should have been awaited");

    // Wait for leader lease to expire
    TypeConfig::sleep(Duration::from_millis(700)).await;

    tracing::info!("--- trigger election on node 2");
    let n2 = router.get_raft_handle(&2)?;
    n2.trigger().elect(false).await?;

    n2.wait(Some(Duration::from_millis(2000)))
        .state(ServerState::Leader, "wait for node 2 to become leader")
        .await?;

    n1.wait(Some(Duration::from_millis(2000)))
        .current_leader(2, "wait for node 1 to see node 2 as leader")
        .await?;

    callbacks.wait(2).await?;

    tracing::info!("--- verify callback future was awaited for new leader");
    let got = callbacks.get();
    assert_eq!(got, vec![(), ()], "callback future should have been awaited twice");

    handle.close().await;

    Ok(())
}
