//! Test clustering behavior.

mod fixtures;

use std::time::Duration;

use actix::prelude::*;
use actix_raft::metrics::{RaftMetrics, State};
use futures::sync::oneshot;

use fixtures::{
    RaftTestController, Node, setup_logger,
    dev::{ExecuteInRaftRouter, RaftRouter, Register},
};

/// Basic lifecycle tests for a three node cluster.
///
/// What does this test cover?
///
/// - The formation of a new cluster where a leader is elected and agreed upon.
/// - When that leader dies, and remains dead until an election timeout, a new leader will be
/// elected by the remaininng nodes.
/// - The new cluster should all agree upon the new leader, and should have a new term.
/// - When the old leader comes back online, it will realize that it is no longer leader, new
/// cluster nodes will reject its heartbeats, and the old leader will become a follower of the
/// new leader.
/// - The new leader generates and commits a new blank log entry to guard against stale writes for
/// when a previous leader had uncommitted entries replicated on some nodes. See end of §8.
///
/// `RUST_LOG=actix_raft,clustering=debug cargo test clustering`
#[test]
fn clustering() {
    setup_logger();
    let sys = System::builder().stop_on_panic(true).name("test").build();

    // Setup test dependencies.
    let net = RaftRouter::new();
    let network = net.start();
    let members = vec![0, 1, 2];
    let node0 = Node::builder(0, network.clone(), members.clone()).build();
    network.do_send(Register{id: 0, addr: node0.addr.clone()});
    let node1 = Node::builder(1, network.clone(), members.clone()).build();
    network.do_send(Register{id: 1, addr: node1.addr.clone()});
    let node2 = Node::builder(2, network.clone(), members.clone()).build();
    network.do_send(Register{id: 2, addr: node2.addr.clone()});

    // Setup test controller and actions.
    let mut ctl = RaftTestController::new(network);
    ctl.register(0, node0.addr.clone()).register(1, node1.addr.clone()).register(2, node2.addr.clone());
    ctl.start_with_test(10, Box::new(|act, ctx| {
        let (tx0, rx0) = oneshot::channel();
        act.network.do_send(ExecuteInRaftRouter(Box::new(move |act, _| {
            let node0: &RaftMetrics = act.metrics.get(&0).unwrap();
            let node1: &RaftMetrics = act.metrics.get(&1).unwrap();
            let node2: &RaftMetrics = act.metrics.get(&2).unwrap();
            let data = vec![node0, node1, node2];

            // General assertions.
            let leader = data.iter().find(|e| &e.state == &State::Leader);
            assert!(leader.is_some(), "Expected leader to exist.");
            let leader_id = leader.unwrap().id;
            assert!(data.iter().all(|e| e.current_leader == Some(leader_id)), "Expected all nodes have the same leader.");
            let term = data.first().unwrap().current_term;
            assert!(data.iter().all(|e| e.current_term == term), "Expected all nodes to be at the same term.");
            // Index of 1 is asserted against here as new leader committ a blank entry.
            assert!(data.iter().all(|e| e.last_log_index == 1), "Expected all nodes have last log index '1'.");
            assert!(data.iter().all(|e| e.last_applied == 1), "Expected all nodes have last applied '1'.");

            // Isolate the current leader on the network.
            act.isolate_node(leader_id);
            let _ = tx0.send((leader_id, term)).unwrap();
        })));

        // Give the cluster some time to elect a new leader. Old leader will remain isolated.
        // Then assert that a new leader has been elected as part of a new term.
        let (tx1, rx1) = oneshot::channel();
        ctx.spawn(fut::wrap_future(rx0).map_err(|err, _: &mut RaftTestController, _| panic!(err))
            .and_then(|old_leader_and_term, _, ctx: &mut Context<RaftTestController>| {
                ctx.run_later(Duration::from_secs(5), move |act, _| {
                    act.network.do_send(ExecuteInRaftRouter(Box::new(move |act, _| {
                        act.restore_node(old_leader_and_term.0);
                        let node0: &RaftMetrics = act.metrics.get(&0).unwrap();
                        let node1: &RaftMetrics = act.metrics.get(&1).unwrap();
                        let node2: &RaftMetrics = act.metrics.get(&2).unwrap();
                        let data = vec![node0, node1, node2];
                        let new_cluster: Vec<_> = data.iter().filter(|e| e.id != old_leader_and_term.0).collect();
                        let old_leader = data.iter().filter(|e| e.id == old_leader_and_term.0).nth(0).unwrap();

                        // Assertions on new cluster.
                        let leader = new_cluster.iter().find(|e| &e.state == &State::Leader);
                        assert!(leader.is_some(), "Expected leader to exist for new cluster.");
                        let leader_id = leader.unwrap().id;
                        assert!(new_cluster.iter().all(|e| e.current_leader == Some(leader_id)), "Expected all new cluster nodes have the same leader.");
                        let term = new_cluster.first().unwrap().current_term;
                        assert!(new_cluster.iter().all(|e| e.current_term == term), "Expected all nodes to be at the same term.");
                        assert!(term != old_leader_and_term.1, "Expected new cluster to have a new term.");

                        // Assertions on old cluster.
                        assert_eq!(old_leader.current_term, old_leader_and_term.1, "Expected old terms match.");
                        assert_eq!(old_leader.current_leader, Some(old_leader_and_term.0), "Expected old leader to still thinks it is leader.");

                        let _ = tx1.send((leader_id, term)).unwrap();
                    })));
                });
                fut::ok(())
            }));

        // Give the old node some time to rejoin, then assert that the new leader remains, new
        // term remains, and the old node is a follower of the new leader.
        ctx.spawn(fut::wrap_future(rx1).map_err(|err, _: &mut RaftTestController, _| panic!(err))
            .and_then(|new_leader_and_term, _, ctx: &mut Context<RaftTestController>| {
                ctx.run_later(Duration::from_secs(5), move |act, _| {
                    act.network.do_send(ExecuteInRaftRouter(Box::new(move |act, _| {
                        let node0: &RaftMetrics = act.metrics.get(&0).unwrap();
                        let node1: &RaftMetrics = act.metrics.get(&1).unwrap();
                        let node2: &RaftMetrics = act.metrics.get(&2).unwrap();
                        let data = vec![node0, node1, node2];

                        // General assertions.
                        let leader_id = data.iter().find(|e| &e.state == &State::Leader)
                            .expect("Expected new leader to remain the same after old node re-joins.").id;
                        assert!(data.iter().all(|e| e.current_leader == Some(leader_id)), "Expected all nodes to have the same leader.");
                        assert!(data.iter().all(|e| e.current_term == new_leader_and_term.1), "Expected all nodes to have same term since last election.");

                        System::current().stop();
                    })));
                });
                fut::ok(())
            }));
    }));

    // Run the test.
    assert!(sys.run().is_ok(), "Error during test.");
}
