//! Test single-node.

mod fixtures;

use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_raft::{
    admin::{InitWithConfig},
    messages::{ClientError, EntryNormal, ResponseMode},
    metrics::{State},
};
use log::{error};
use tokio_timer::Delay;

use fixtures::{
    Payload, RaftTestController, Node, setup_logger,
    dev::{ExecuteInRaftRouter, GetCurrentLeader, RaftRouter, Register},
    memory_storage::MemoryStorageData,
};

/// Single-node test.
///
/// What does this test cover?
///
/// - Start a single Raft node.
/// - Issue the `InitWithConfig` command.
/// - Assert that the cluster was able to come online, elect a leader and maintain a stable state.
///
/// `RUST_LOG=actix_raft,singlenode=debug cargo test singlenode`
#[test]
fn singlenode() {
    setup_logger();
    let sys = System::builder().stop_on_panic(true).name("test").build();

    // Setup test dependencies.
    let net = RaftRouter::new();
    let network = net.start();
    let node0 = Node::builder(0, network.clone(), vec![0]).build();
    network.do_send(Register{id: 0, addr: node0.addr.clone()});

    // Setup test controller and actions.
    let mut ctl = RaftTestController::new(network);
    ctl.register(0, node0.addr.clone());
    ctl.start_with_test(10, Box::new(|act, ctx| {
        // Assert that all nodes are in NonVoter state with index 0.
        let task = fut::wrap_future(act.network.send(ExecuteInRaftRouter(Box::new(move |act, _| {
            for node in act.metrics.values() {
                assert!(node.state == State::NonVoter, "Expected all nodes to be in NonVoter state at beginning of test.");
                assert_eq!(node.last_log_index, 0, "Expected all nodes to be at index 0 at beginning of test.");
            }
        }))))
            .map_err(|err, _: &mut RaftTestController, _| panic!(err)).and_then(|res, _, _| fut::result(res))

            // Issue the InitWithConfig command to all nodes in parallel.
            .and_then(|_, act, _| {
                let task = act.nodes.get(&0).expect("Expected node 0 to be registered with test controller.")
                    .send(InitWithConfig::new(vec![0]))
                    .map_err(|err| panic!(err))
                    .and_then(|res| res)
                    .map_err(|err| panic!(err));
                fut::wrap_future(task)
            })

            // Delay for a bit and then write data to the cluster.
            .and_then(|_, _, _| fut::wrap_future(Delay::new(Instant::now() + Duration::from_secs(5)).map_err(|_| ())))
            .and_then(|_, act, ctx| act.write_data(ctx))

            // Assert against the state of the cluster.
            .and_then(|_, _, _| fut::wrap_future(Delay::new(Instant::now() + Duration::from_secs(2)).map_err(|_| ())))
            .and_then(|_, act, _| {
                fut::wrap_future(act.network.send(ExecuteInRaftRouter(Box::new(move |act, _| {
                    // Cluster should have an elected leader.
                    let leader = act.metrics.values().find(|e| &e.state == &State::Leader)
                        .expect("Expected cluster to have an elected leader.");

                    // All nodes should agree upon the leader, term, index, applied & config.
                    assert!(act.metrics.values().all(|node| node.current_leader == Some(leader.id)), "Expected all nodes to agree upon current leader.");
                    assert!(act.metrics.values().all(|node| node.current_term == leader.current_term), "Expected all nodes to agree upon current term.");
                    assert!(act.metrics.values().all(|node| node.last_log_index == leader.last_log_index), "Expected all nodes to be at the same index.");
                    assert!(act.metrics.values().all(|node| node.membership_config == leader.membership_config), "Expected all nodes to have same cluster config.");

                    System::current().stop();
                }))))
                .map_err(|err, _, _| panic!(err)).and_then(|res, _, _| fut::result(res))
            })

            .map_err(|err, _, _| panic!("Failure during test. {:?}", err));

        ctx.spawn(task);
    }));

    // Run the test.
    assert!(sys.run().is_ok(), "Error during test.");
}

impl RaftTestController {
    fn write_data(&mut self, _: &mut Context<Self>) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        fut::wrap_future(self.network.send(GetCurrentLeader))
            .map_err(|_, _: &mut Self, _| panic!("Failed to get current leader."))
            .and_then(|res, _, _| fut::result(res))
            .and_then(|current_leader, act, _| {
                let num_requests = 100;
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
