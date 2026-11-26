use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::ServerState;
use openraft::StepDownPolicy;
use openraft::Vote;
use openraft::WatchChangeHandle;
use openraft::async_runtime::WatchReceiver;
use openraft::async_runtime::WatchSender;
use openraft::type_config::TypeConfigExt;
use openraft::vote::RaftLeaderId;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemRaft;
use crate::fixtures::Node;
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

/// A leader restarted without restoring its leadership follows with its committed vote: no
/// `start` is called for that vote, only once the node is elected again.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn on_leader_change_after_passive_restart() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        enable_leader_restore: Some(false),
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let log_index = router.new_cluster([0], []).await?;

    tracing::info!(
        log_index,
        "--- restart node-0: it follows with its committed vote of term 1"
    );
    {
        let Node {
            log_store,
            state_machine,
            ..
        } = router.stop_node(0).await?;
        router.new_raft_node_with_sto(0, log_store, state_machine).await?;

        let want = Vote::new_committed(1, 0);
        router
            .wait(&0)
            .metrics(
                |m| m.state == ServerState::Follower && m.vote == want,
                "node-0 follows with its committed vote",
            )
            .await?;
    }

    tracing::info!(
        log_index,
        "--- watch node-0, then elect it: the first callback is `start` of term 2"
    );
    {
        let n0 = router.get_raft_handle(&0)?;
        let (events, mut handle) = watch_leader_events(&n0);

        n0.trigger().elect(false).await?;

        let got = events.wait(1).await?;
        assert_eq!(vec![("start", 2, 0)], got);

        handle.close().await;
    }

    Ok(())
}

/// A removed leader keeps its committed vote when it steps down to a learner: `stop` is called
/// when it steps down.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn on_leader_change_when_removed_leader_steps_down() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        removed_leader_step_down: StepDownPolicy::Never,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster with voters 0,1,2");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;

    let n0 = router.get_raft_handle(&0)?;
    let (events, mut handle) = watch_leader_events(&n0);

    tracing::info!(log_index, "--- node-0 leads in term 1: `start` is called");
    {
        let got = events.wait(1).await?;
        assert_eq!(vec![("start", 1, 0)], got);
    }

    tracing::info!(
        log_index,
        "--- remove leader node-0: it keeps leading until it is told to step down"
    );
    {
        n0.change_membership([1, 2], false).await?;
        log_index += 1;

        let metrics = router.wait(&0).applied_index(Some(log_index), "node-0 applies its removal").await?;
        assert_eq!(ServerState::Leader, metrics.state, "node-0 keeps leading");
    }

    tracing::info!(
        log_index,
        "--- step node-0 down: it keeps its committed vote of term 1, and `stop` is called"
    );
    {
        n0.trigger().refresh_server_state(None, None).await?;
        let metrics = router.wait(&0).state(ServerState::Learner, "node-0 steps down").await?;
        assert_eq!(Vote::new_committed(1, 0), metrics.vote);

        let got = events.wait(2).await?;
        assert_eq!(vec![("start", 1, 0), ("stop", 1, 0)], got);
    }

    handle.close().await;

    Ok(())
}

/// A leader whose leadership transfer times out campaigns in a new term: `stop` is called while it
/// is a candidate, and `start` once it leads again.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn on_leader_change_when_leader_campaigns_again() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster with voters 0,1,2");
    let log_index = router.new_cluster([0, 1, 2], []).await?;

    let n0 = router.get_raft_handle(&0)?;
    let (events, mut handle) = watch_leader_events(&n0);

    tracing::info!(log_index, "--- node-0 leads in term 1: `start` is called");
    {
        let got = events.wait(1).await?;
        assert_eq!(vec![("start", 1, 0)], got);
    }

    tracing::info!(
        log_index,
        "--- cut node-0 off and transfer to absent node-3: `stop` is called once node-0 campaigns"
    );
    {
        router.set_network_error(0, true);
        n0.trigger().transfer_leader(3).await?;
        n0.runtime_config().elect(true);

        router.wait(&0).state(ServerState::Candidate, "node-0 campaigns in a new term").await?;

        let got = events.wait(2).await?;
        assert_eq!(vec![("start", 1, 0), ("stop", 1, 0)], got);
    }

    tracing::info!(
        log_index,
        "--- reconnect node-0 and elect it: `start` is called for its new term"
    );
    {
        n0.runtime_config().elect(false);
        router.set_network_error(0, false);
        n0.trigger().elect(false).await?;

        let metrics = router
            .wait(&0)
            .metrics(
                |m| m.state == ServerState::Leader && m.vote.is_committed(),
                "node-0 leads again",
            )
            .await?;
        let term = metrics.vote.leader_id().term();
        assert!(term > 1, "node-0 leads in a new term: {term}");

        let got = events.wait(3).await?;
        assert_eq!(vec![("start", 1, 0), ("stop", 1, 0), ("start", term, 0)], got);
    }

    handle.close().await;

    Ok(())
}

/// A leader that leads again in a new term, with no other role seen by the watch in between:
/// `start` for the new term is called only after the future of `stop` for the old term completes.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn on_leader_change_calls_start_after_stop_completes() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let mut log_index = router.new_cluster([0], []).await?;

    let n0 = router.get_raft_handle(&0)?;
    let events = Events::<LeaderEvent>::default();
    let (gate_tx, gate_rx) = TypeConfig::watch_channel(false);

    tracing::info!(
        log_index,
        "--- watch node-0: `start` records its call at once, then waits for the gate; \
         `stop` records its call when its future completes"
    );
    let mut handle = {
        let started = events.clone();
        let stopped = events.clone();
        let handle = n0.on_leader_change(
            move |leader_id| {
                started.push(("start", leader_id.term(), *leader_id.node_id()));
                let mut gate = gate_rx.clone();
                async move {
                    let _ = gate.wait_until_ge(&true).await;
                }
            },
            move |leader_id| {
                let stopped = stopped.clone();
                async move {
                    TypeConfig::yield_now().await;
                    stopped.push(("stop", leader_id.term(), *leader_id.node_id()));
                }
            },
        );

        let got = events.wait(1).await?;
        assert_eq!(vec![("start", 1, 0)], got);

        handle
    };

    tracing::info!(
        log_index,
        "--- while `start` for term 1 waits, transfer leadership to absent node-1: \
         elections are on, so node-0 campaigns once the transfer times out, and leads in term 2"
    );
    {
        n0.trigger().transfer_leader(1).await?;

        let want = Vote::new_committed(2, 0);
        router
            .wait(&0)
            .metrics(
                |m| m.state == ServerState::Leader && m.vote == want,
                "node-0 leads in term 2",
            )
            .await?;
        log_index += 1;
    }

    tracing::info!(
        log_index,
        "--- open the gate: the watch sees term 2 right after term 1, \
         and calls `start` for term 2 once `stop` for term 1 completes"
    );
    {
        gate_tx.send(true)?;

        let got = events.wait(3).await?;
        assert_eq!(vec![("start", 1, 0), ("stop", 1, 0), ("start", 2, 0)], got);
    }

    handle.close().await;

    Ok(())
}

/// A leader that shuts down gets `stop` for its leadership.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn on_leader_change_when_leader_shuts_down() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let log_index = router.new_cluster([0], []).await?;

    let n0 = router.get_raft_handle(&0)?;
    let (events, mut handle) = watch_leader_events(&n0);

    tracing::info!(log_index, "--- node-0 leads in term 1: `start` is called");
    {
        let got = events.wait(1).await?;
        assert_eq!(vec![("start", 1, 0)], got);
    }

    tracing::info!(log_index, "--- shut node-0 down: `stop` is called");
    {
        n0.shutdown().await?;

        let got = events.wait(2).await?;
        assert_eq!(vec![("start", 1, 0), ("stop", 1, 0)], got);
    }

    handle.close().await;

    Ok(())
}

/// The watch holds no `Raft` handle: dropping every handle of a leader stops the node, and `stop`
/// is called.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn on_leader_change_when_every_handle_is_dropped() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let log_index = router.new_cluster([0], []).await?;

    let (events, mut handle) = {
        let n0 = router.get_raft_handle(&0)?;
        watch_leader_events(&n0)
    };

    tracing::info!(log_index, "--- node-0 leads in term 1: `start` is called");
    {
        let got = events.wait(1).await?;
        assert_eq!(vec![("start", 1, 0)], got);
    }

    tracing::info!(
        log_index,
        "--- detach node-0 and drop its last handle: the node stops, and `stop` is called"
    );
    {
        let node = router.detach_node(0).expect("node-0 is registered");
        drop(node);

        let got = events.wait(2).await?;
        assert_eq!(vec![("start", 1, 0), ("stop", 1, 0)], got);
    }

    handle.close().await;

    Ok(())
}

/// A core that panics closes the metrics channel without reporting `Shutdown`: `stop` is called
/// when the channel closes.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn on_leader_change_when_core_panics() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let log_index = router.new_cluster([0], []).await?;

    let n0 = router.get_raft_handle(&0)?;
    let (events, mut handle) = watch_leader_events(&n0);

    tracing::info!(log_index, "--- node-0 leads in term 1: `start` is called");
    {
        let got = events.wait(1).await?;
        assert_eq!(vec![("start", 1, 0)], got);
    }

    tracing::info!(
        log_index,
        "--- panic the core of node-0: its metrics channel closes, and `stop` is called"
    );
    {
        router.external_request(0, |_| panic!("panic the core of node-0")).await?;

        let got = events.wait(2).await?;
        assert_eq!(vec![("start", 1, 0), ("stop", 1, 0)], got);

        let state = n0.metrics().borrow_watched().state;
        assert_eq!(
            ServerState::Leader,
            state,
            "the core reports no `Shutdown` before it closes"
        );
    }

    handle.close().await;

    Ok(())
}

/// An `on_leader_change()` callback: `"start"` or `"stop"`, with the term and the node ID of the
/// leader ID it received.
type LeaderEvent = (&'static str, u64, u64);

/// Register `on_leader_change()` on `raft`, and record its callbacks in the order they are called.
fn watch_leader_events(raft: &MemRaft) -> (Events<LeaderEvent>, WatchChangeHandle<TypeConfig>) {
    let events = Events::default();

    let started = events.clone();
    let stopped = events.clone();
    let handle = raft.on_leader_change(
        move |leader_id| {
            started.push(("start", leader_id.term(), *leader_id.node_id()));
            async {}
        },
        move |leader_id| {
            stopped.push(("stop", leader_id.term(), *leader_id.node_id()));
            async {}
        },
    );

    (events, handle)
}
