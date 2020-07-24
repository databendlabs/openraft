//! Test client data writing behavior.

mod fixtures;

use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_raft::metrics::{RaftMetrics, State};
use tokio_timer::Delay;

use fixtures::{
    ClientRequest, RaftTestController, Node, setup_logger,
    dev::{ExecuteInRaftRouter, GetCurrentLeader, RaftRouter, Register},
    memory_storage::{GetCurrentState},
};

/// Basic lifecycle tests for a three node cluster.
///
/// What does this test cover?
///
/// - The client request path. Client data should be appended to the log, replicated & then
/// applied to the state machine.
/// - Client requests should be processed by the leader, and if the node
/// is not the leader, it will return a forwarding error with the information on the leader.
/// - If the leader dies during client write load, the clients should receive forwarding info,
/// and the new leader should be able to handle client requests as normal.
/// - after all data has been written, all nodes should have identical logs & state machines.
///
/// `RUST_LOG=actix_raft,client_writes=debug cargo test client_writes`
#[test]
fn client_writes() {
    setup_logger();
    let sys = System::builder().stop_on_panic(true).name("test").build();

    // Setup test dependencies.
    let netarb = Arbiter::new();
    let network = RaftRouter::start_in_arbiter(&netarb, |_| RaftRouter::new());
    let members = vec![0, 1, 2];
    let node0 = Node::builder(0, network.clone(), members.clone()).build();
    network.do_send(Register{id: 0, addr: node0.addr.clone()});
    let node1 = Node::builder(1, network.clone(), members.clone()).build();
    network.do_send(Register{id: 1, addr: node1.addr.clone()});
    let node2 = Node::builder(2, network.clone(), members.clone()).build();
    network.do_send(Register{id: 2, addr: node2.addr.clone()});
    let (storage0, storage1, storage2) = (node0.storage.clone(), node1.storage.clone(), node2.storage.clone());

    // Setup test controller and actions.
    let mut ctl = RaftTestController::new(network);
    ctl.register(0, node0.addr.clone()).register(1, node1.addr.clone()).register(2, node2.addr.clone());
    ctl.start_with_test(10,  Box::new(|act, ctx| {
        // Get the current leader.
        ctx.spawn(fut::wrap_future(act.network.send(GetCurrentLeader))
            .map_err(|_, _: &mut RaftTestController, _| panic!("Failed to get current leader."))
            .and_then(|res, _, _| fut::result(res)).and_then(|leader_opt, _, _| {
                let leader = leader_opt.expect("Expected the cluster to have elected a leader.");
                fut::ok(leader)
            })
            // Send 100 requests as fast as possible to the current leader & give the cluster 1 second to do work.
            .and_then(|leader, _, ctx| {
                for idx in 0..100 {
                    ctx.notify(ClientRequest{payload: idx, current_leader: Some(leader), cb: None});
                }
                fut::wrap_future(Delay::new(Instant::now() + Duration::from_secs(1))).map_err(|_, _, _| ())
                    .map(move |_, _, _| leader)
            })

            // Bring down the current leader.
            .and_then(|leader, act, _| {
                act.network.do_send(ExecuteInRaftRouter(Box::new(move |act, _| act.isolate_node(leader))));
                fut::wrap_future(Delay::new(Instant::now() + Duration::from_secs(2))).map_err(|_, _, _| ())
                    .map(move |_, _, _| leader)
            })

            // Get new leader ID.
            .and_then(|old_leader, act, _| {
                fut::wrap_future(act.network.send(GetCurrentLeader))
                .map_err(|_, _: &mut RaftTestController, _| panic!("Failed to get current leader."))
                .and_then(|res, _, _| fut::result(res))
                .map(move |current_leader, _, _| (current_leader, old_leader))
            })

            // Restore old node, send 100 more requests & wait for 3 second for the system to process.
            .and_then(|(current_leader, old_leader), act, ctx| {
                act.network.do_send(ExecuteInRaftRouter(Box::new(move |act, _| act.restore_node(old_leader))));
                for idx in 0..100 {
                    ctx.notify(ClientRequest{payload: idx, current_leader, cb: None});
                }
                fut::wrap_future(Delay::new(Instant::now() + Duration::from_secs(10))).map_err(|_, _, _| ())
            })

            // Assert that all nodes are up, same leader, same final state from the standpoint of the metrics.
            .and_then(|_, act, _| {
                act.network.do_send(ExecuteInRaftRouter(Box::new(|act, _| {
                    let node0: &RaftMetrics = act.metrics.get(&0).unwrap();
                    let node1: &RaftMetrics = act.metrics.get(&1).unwrap();
                    let node2: &RaftMetrics = act.metrics.get(&2).unwrap();
                    let data = vec![node0, node1, node2];
                    let leader = data.iter().find(|e| &e.state == &State::Leader).expect("Expected leader to exist.");
                    assert!(data.iter().all(|e| e.current_leader == Some(leader.id)), "Expected all nodes have the same leader.");
                    assert!(data.iter().all(|e| e.current_term == leader.current_term), "Expected all nodes to be at the same term.");
                    assert!(data.iter().all(|e| e.last_log_index == leader.last_log_index), "Expected all nodes have last log index.");
                    assert!(data.iter().all(|e| e.last_applied == leader.last_applied), "Expected all nodes have the same last applied value.");
                })));
                fut::ok(())
            })

            // Assert that the state of all Raft node storage engines are the same.
            .and_then(move |_, _, _| {
                // Callback pyramid of death to fetch storage data.
                fut::wrap_future(storage0.send(GetCurrentState)).map_err(|err, _: &mut RaftTestController, _| panic!(err))
                    .and_then(|res, _, _| fut::result(res)).and_then(move |s0, _, _| {
                        fut::wrap_future(storage1.send(GetCurrentState)).map_err(|err, _, _| panic!(err))
                            .and_then(|res, _, _| fut::result(res)).and_then(move |s1, _, _| {
                                fut::wrap_future(storage2.send(GetCurrentState)).map_err(|err, _, _| panic!(err))
                                    .and_then(|res, _, _| fut::result(res)).and_then(move |s2, _, _| {
                                        fut::ok((s0, s1, s2))
                                    })
                            })
                    })
                    // We've got our storage data, assert against it.
                    .and_then(|(s0, s1, s2), _, _| {
                        // Hard state.
                        assert_eq!(s0.hs.current_term, s1.hs.current_term, "Expected hs current term for nodes 0 and 1 to be equal.");
                        assert_eq!(s0.hs.current_term, s2.hs.current_term, "Expected hs current term for nodes 0 and 2 to be equal.");
                        assert_eq!(s0.hs.membership, s1.hs.membership, "Expected hs membership for nodes 0 and 1 to be equal.");
                        assert_eq!(s0.hs.membership, s2.hs.membership, "Expected hs membership for nodes 0 and 2 to be equal.");
                        // Log.
                        assert_eq!(s0.log, s1.log, "Expected log for nodes 0 and 1 to be equal.");
                        assert_eq!(s0.log, s2.log, "Expected log for nodes 0 and 2 to be equal.");
                        // Snapshot data.
                        assert_eq!(s0.snapshot_data, s1.snapshot_data, "Expected snapshot_data for nodes 0 and 1 to be equal.");
                        assert_eq!(s0.snapshot_data, s2.snapshot_data, "Expected snapshot_data for nodes 0 and 2 to be equal.");
                        // State machinen data.
                        assert_eq!(s0.state_machine, s1.state_machine, "Expected state machines for nodes 0 and 1 to be equal.");
                        assert_eq!(s0.state_machine, s2.state_machine, "Expected state machines for nodes 0 and 2 to be equal.");
                        fut::ok(())
                    })
                    .and_then(|_, _, ctx| {
                        ctx.run_later(Duration::from_secs(2), |_, _| System::current().stop());
                        fut::ok(())
                    })
            })
            .map_err(|err, _, _| panic!("Failure during test. {:?}", err)));
    }));

    // Run the test.
    assert!(sys.run().is_ok(), "Error during test.");
}
