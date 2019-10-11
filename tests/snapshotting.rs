//! Test snapshotting behavior.

mod fixtures;

use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_raft::{
    NodeId,
    config::DEFAULT_LOGS_SINCE_LAST,
    messages::{ClientError, EntryNormal, ResponseMode},
    metrics::{RaftMetrics, State},
};
use log::{error};
use tokio_timer::Delay;

use fixtures::{
    Node, RaftTestController, Payload, setup_logger,
    dev::{ExecuteInRaftRouter, GetCurrentLeader, RaftRouter, Register},
    memory_storage::{GetCurrentState, MemoryStorageData},
};

/// Basic lifecycle tests for a three node cluster.
///
/// What does this test cover?
///
/// TODO: update this.
///
/// `RUST_LOG=actix_raft,snapshotting=debug cargo test snapshotting`
#[test]
fn snapshotting() {
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
    let (storage0, storage1, storage2) = (node0.storage.clone(), node1.storage.clone(), node2.storage.clone());

    // Setup test controller and actions.
    let mut ctl = RaftTestController::new(network);
    ctl.register(0, node0.addr.clone()).register(1, node1.addr.clone()).register(2, node2.addr.clone());
    ctl.start_with_test(10, Box::new(|act, ctx| {
        // Isolate the current leader.
        let task = act.isolate_leader(ctx)
            // Wait for new leader to be elected.
            .and_then(|old_leader, _, _| {
                fut::wrap_future(Delay::new(Instant::now() + Duration::from_secs(5))).map_err(|_, _, _| ())
                    .map(move |_, _, _| old_leader)
            })
            // Write enough data to trigger a snapshot. Won't proceed until data is finished writing.
            .and_then(|old_leader, act, ctx| act.write_above_snapshot_threshold(ctx).map(move |_, _, _| old_leader))
            // Restore old node.
            .and_then(|old_leader, act, _| {
                act.network.do_send(ExecuteInRaftRouter(Box::new(move |act, _| act.restore_node(old_leader))));
                fut::ok(())
            })
            // Wait for old leader to be brought up-to-speed with a snapshot. This can take some
            // time depending on the system. 15 seconds should be way more than enough. Keep it
            // high to reduce transient test failures.
            .and_then(|_, _, _| fut::wrap_future(Delay::new(Instant::now() + Duration::from_secs(15))).map_err(|_, _, _| ()))

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
            .map_err(|err, _, _| panic!("Failure during test. {:?}", err));
        ctx.spawn(task);
    }));

    // Run the test.
    assert!(sys.run().is_ok(), "Error during test.");
}

impl RaftTestController {
    fn isolate_leader(&mut self, _: &mut Context<Self>) -> impl ActorFuture<Actor=Self, Item=NodeId, Error=()> {
        fut::wrap_future(self.network.send(GetCurrentLeader))
            .map_err(|_, _: &mut Self, _| panic!("Failed to get current leader."))
            .and_then(|res, _, _| fut::result(res))
            .and_then(|current_leader, act, _| {
                let leader = current_leader.expect("Expected a leader to have been elected.");
                act.network.do_send(ExecuteInRaftRouter(Box::new(move |act, _| act.isolate_node(leader))));
                fut::ok(leader)
            })
    }

    fn write_above_snapshot_threshold(&mut self, _: &mut Context<Self>) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        fut::wrap_future(self.network.send(GetCurrentLeader))
            .map_err(|_, _: &mut Self, _| panic!("Failed to get current leader."))
            .and_then(|res, _, _| fut::result(res))
            .and_then(|current_leader, act, _| {
                let num_requests = DEFAULT_LOGS_SINCE_LAST + 10;
                let leader_id = current_leader.expect("Expected to find a current cluster leader for writing client requests.");
                let addr = act.nodes.get(&leader_id).expect("Expected leader to be present it RaftTestController's nodes map.");
                let leader = addr.clone();

                fut::wrap_stream(futures::stream::iter_ok(0..num_requests))
                    .and_then(move |data, _, _| {
                        let entry = EntryNormal{data: MemoryStorageData{data: data.to_string().into_bytes()}};
                        let payload = Payload::new(entry, ResponseMode::Applied);
                        fut::wrap_future(leader.clone().send(payload))
                            .map_err(|_, _, _| ClientError::Internal)
                            .and_then(|res, _, _| fut::result(res))
                            .then(move |res, _, _| match res {
                                Ok(_) => fut::ok(()),
                                Err(err) => {
                                    error!("TEST: Error during client request. {}", err);
                                    fut::err(())
                                }
                            })
                    })
                    .finish()
            })
    }
}
