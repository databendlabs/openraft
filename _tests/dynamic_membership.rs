//! Test dynamic membership.

mod fixtures;

use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_raft::{
    admin::{ProposeConfigChange},
    messages::{ClientError, EntryNormal, ResponseMode},
    metrics::{State},
};
use log::{error};
use tokio_timer::Delay;

use fixtures::{
    Payload, RaftTestController, Node, setup_logger,
    dev::{ExecuteInRaftRouter, GetCurrentLeader, RaftRouter, Register, RemoveNodeFromCluster},
    memory_storage::MemoryStorageData,
};

/// Dynamic cluster membership test.
///
/// What does this test cover?
///
/// - Bring a standard 3 node cluster up to an operational status.
/// - Propose a config change which will add a new node and which will remove the current leader.
/// - The request should succeed.
/// - The request should be committed to the cluster.
/// - The new node should be brought up-to-speed with the cluster and should be a follower.
/// - The old node should be in a non-voter state and should no longer be receiving data written
/// to the cluster. A new leader should have been elected.
///
/// `RUST_LOG=actix_raft,dynamic_membership=debug cargo test dynamic_membership`
#[test]
fn dynamic_membership() {
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
        let task = act.write_data(ctx)
            // Data has been writtent to new cluster, get the ID of the current leader.
            .and_then(|_, act, _| {
                fut::wrap_future(act.network.send(GetCurrentLeader))
                    .map_err(|_, _, _| panic!("Failed to get current leader."))
                    .and_then(|res, _, _| fut::result(res)).and_then(|leader_opt, _, _| {
                        let leader = leader_opt.expect("Expected the cluster to have elected a leader.");
                        fut::ok(leader)
                    })
            })
            // Propose a config change to the cluster adding node3 and removing the current leader.
            .and_then(|leader_id, act, _| {
                let addr = act.nodes.get(&leader_id).expect("Expected leader to be present it RaftTestController's nodes map.");
                let leader = addr.clone();

                // Prep an additional node to be added to the cluster later.
                let node3 = Node::builder(3, act.network.clone(), vec![3]).build();
                act.register(3, node3.addr.clone());
                act.network.do_send(Register{id: 3, addr: node3.addr.clone()});

                fut::wrap_future(leader.send(ProposeConfigChange::new(vec![3], vec![leader_id])))
                    .map_err(|_, _: &mut RaftTestController, _| panic!("Messaging error while proposing config change to leader."))
                    .and_then(|res, _, _| fut::ok(res.expect("Failed to propose config change to leader.")))
                    .map(move |_, _, _| leader_id)
            })
            // Delay for a bit to ensure we can target a new leader in the test.
            .and_then(|leader_id, _, _| fut::wrap_future(Delay::new(Instant::now() + Duration::from_secs(6)))
                .map_err(|_, _, _| ())
                .map(move |_, _, _| leader_id))
            // Remove old node.
            .and_then(|leader_id, act, _| {
                fut::wrap_future(act.network.send(RemoveNodeFromCluster{id: leader_id}))
                    .map_err(|_, _: &mut RaftTestController, _| panic!("Messaging error while attempting to remove old node."))
                    .and_then(|res, _, _| fut::result(res))
                    .map_err(|err, _, _| panic!(err))
                    .map(move |_, _, _| leader_id)
            })
            // Write some additional data to the new leader.
            .and_then(|old_leader_id, act, ctx| act.write_data(ctx).map(move |_, _, _| old_leader_id))
            .and_then(|old_leader_id, _, _| fut::wrap_future(Delay::new(Instant::now() + Duration::from_secs(6))
                .map_err(|_| ()))
                .map(move |_, _, _| old_leader_id))

            // Assert against the state of all nodes in the cluster.
            .and_then(|old_leader_id, act, _| {
                act.network.do_send(ExecuteInRaftRouter(Box::new(move |act, _| {
                    let leader = act.metrics.values().find(|e| &e.state == &State::Leader).expect("Expected cluster to have active leader.");
                    assert_eq!(leader.membership_config.members.len(), 3, "Expected a three node cluster.");
                    assert_eq!(leader.membership_config.non_voters.len(), 0, "Expected no non-voters in cluster.");
                    assert_eq!(leader.membership_config.removing.len(), 0, "Expected no nodes to be scheduled for removal.");
                    assert_eq!(leader.last_log_index, leader.last_applied, "Expected leader to have matching last log & last applied indices.");
                    assert!(leader.last_log_index > 200, "Expected leader to have committed more than 200 entries.");

                    for nodeid in leader.membership_config.members.iter().filter(|e| *e != &leader.id) {
                        let node = act.metrics.get(nodeid).expect("Expected to find metrics entry for cluster member.");
                        assert_eq!(node.membership_config, leader.membership_config, "Expected all cluster members to have matching config.");
                        assert_eq!(node.last_log_index, leader.last_log_index, "Expected all cluster members to have matching last log index.");
                        assert_eq!(node.last_applied, leader.last_applied, "Expected all cluster members to have matching last applied index.");
                        assert_eq!(node.current_leader, Some(leader.id), "Expected all cluster members to have same leader.");
                    }

                    // The leader which stepped down should be in NonVoter state and should have
                    // an index which is less than 200.
                    let old_leader = act.metrics.values().find(|e| &e.id == &old_leader_id).expect("Expected old leader's metrics to be present.");
                    assert!(old_leader.last_log_index < 200, "Expected old leader last index to be < 200 (to have no longer received replication events).");
                    assert!(old_leader.last_applied < 200, "Expected old leader last index to be < 200 (to have no longer received replication events).");
                    assert_eq!(old_leader.membership_config, leader.membership_config, "Expected old leader to have matching config.");
                    assert_eq!(old_leader.current_leader, None, "Expected old leader to have current leader set to None.");
                    assert!(old_leader.state == State::NonVoter, "Expected old leader to be in state NonVoter.");

                    System::current().stop();
                })));
                fut::ok(())
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
