//! Test client data writing behavior.

mod fixtures;

use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_raft::{
    dev::{ExecuteInRaftRouter, RaftRouter, Register},
    memory_storage::{GetCurrentState, MemoryStorageError},
    messages::{ClientPayload, ClientError, EntryNormal, ResponseMode},
    metrics::{RaftMetrics, State},
};
use env_logger;
use futures::{
    stream::iter_ok,
    sync::oneshot,
};
use log::{debug, error};
use tokio::timer::Delay;

use fixtures::{RaftTestController, new_raft_node};

type Payload = ClientPayload<MemoryStorageError>;

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
///
/// Run with `RUST_LOG=actix_raft,clustering=debug cargo test` to see detailed logs.
#[test]
fn client_data_writing() {
    let _ = env_logger::try_init();
    let sys = System::builder().stop_on_panic(true).name("test").build();

    // Setup test dependencies.
    let netarb = Arbiter::new();
    let network = RaftRouter::start_in_arbiter(&netarb, |_| RaftRouter::new());
    let members = vec![0, 1, 2];
    let node0 = new_raft_node(0, network.clone(), members.clone(), 1);
    network.do_send(Register{id: 0, addr: node0.addr.clone()});
    let node1 = new_raft_node(1, network.clone(), members.clone(), 1);
    network.do_send(Register{id: 1, addr: node1.addr.clone()});
    let node2 = new_raft_node(2, network.clone(), members.clone(), 1);
    network.do_send(Register{id: 2, addr: node2.addr.clone()});
    let (storage0, storage1, storage2) = (node0.storage.clone(), node1.storage.clone(), node2.storage.clone());

    // Setup test controller and actions.
    let mut ctl = RaftTestController::new(network);
    ctl.register(0, node0.addr.clone()).register(1, node1.addr.clone()).register(2, node2.addr.clone());
    ctl.start_with_test(2,  Box::new(|act, ctx| {
        // Cluster should be online (other tests cover this). Find the leader.
        let (tx0, rx0) = oneshot::channel();
        act.network.do_send(ExecuteInRaftRouter(Box::new(move |act, _| {
            let node0: &RaftMetrics = act.metrics.get(&0).unwrap();
            let node1: &RaftMetrics = act.metrics.get(&1).unwrap();
            let node2: &RaftMetrics = act.metrics.get(&2).unwrap();
            let data = vec![node0, node1, node2];
            let leader = data.iter().find(|e| &e.state == &State::Leader).expect("Expected leader to exist.").id;
            let _ = tx0.send(leader).unwrap();
        })));

        // Wait until cluster is ready, then begin testing.
        ctx.spawn(fut::wrap_future(rx0).map_err(|err, _: &mut RaftTestController, _| panic!(err))
            // Begin writing data. Tests forwarding, recovery from leader failure, restoration & consistency.
            .and_then(|leader, act, ctx| act.start_writing_client_data_indirect(ctx, leader))
            // Wait for 2 seconds to ensure final metrics come in.
            .and_then(|_, _, _| {
                fut::wrap_future(Delay::new(Instant::now() + Duration::from_secs(2)))
                    .map_err(|_, _, _| ())
                    .then(|_, _, _| fut::ok(()))
            })
            // Assert that all nodes are up, same leader, same final state from the standpoint of the metrics.
            // Assert the all nodes storage engines have identical data.
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
                        assert_eq!(s0.hs.current_term, s1.hs.current_term, "Expected hs for nodes 0 and 1 to be equal.");
                        assert_eq!(s0.hs.current_term, s2.hs.current_term, "Expected hs for nodes 0 and 2 to be equal.");
                        assert_eq!(s0.hs.members, s1.hs.members, "Expected hs for nodes 0 and 1 to be equal.");
                        assert_eq!(s0.hs.members, s2.hs.members, "Expected hs for nodes 0 and 2 to be equal.");
                        // Log.
                        let _ = std::fs::write("./log-s0.txt", format!("{:?}", s0.log)).expect("Expected to be able to dump storage file.");
                        let _ = std::fs::write("./log-s1.txt", format!("{:?}", s1.log)).expect("Expected to be able to dump storage file.");
                        let _ = std::fs::write("./log-s2.txt", format!("{:?}", s2.log)).expect("Expected to be able to dump storage file.");
                        assert_eq!(s0.log, s1.log, "Expected log for nodes 0 and 1 to be equal.");
                        assert_eq!(s0.log, s2.log, "Expected log for nodes 0 and 2 to be equal.");
                        // Snapshot data.
                        assert_eq!(s0.snapshot_data, s1.snapshot_data, "Expected snapshot_data for nodes 0 and 1 to be equal.");
                        assert_eq!(s0.snapshot_data, s2.snapshot_data, "Expected snapshot_data for nodes 0 and 2 to be equal.");
                        // State machinen data.
                        let _ = std::fs::write("./sm-s0.txt", format!("{:?}", s0.state_machine)).expect("Expected to be able to dump storage file.");
                        let _ = std::fs::write("./sm-s1.txt", format!("{:?}", s1.state_machine)).expect("Expected to be able to dump storage file.");
                        let _ = std::fs::write("./sm-s2.txt", format!("{:?}", s2.state_machine)).expect("Expected to be able to dump storage file.");
                        assert_eq!(s0.state_machine, s1.state_machine, "Expected state machines for nodes 0 and 1 to be equal.");
                        assert_eq!(s0.state_machine, s2.state_machine, "Expected state machines for nodes 0 and 2 to be equal.");
                        fut::ok(())
                    })
                    .and_then(|_, _, _| {
                        System::current().stop();
                        fut::ok(())
                    })
            }));
    }));

    // Run the test.
    assert!(sys.run().is_ok(), "Error during test.");
}

impl RaftTestController {
    /// Begin writing client request data to the cluster.
    ///
    /// This routine does the following:
    ///
    /// - starts off writing data to a follower node so that it will be forwarded to the leader.
    /// - brings down the leader after some number of logs are written.
    /// - restores the old leader to ensure it integrates back into the cluster.
    /// - finishies writing new logs.
    fn start_writing_client_data_indirect(&mut self, _: &mut Context<Self>, leader: u64) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        debug!("Starting to write client data.");
        let (id, addr) = self.nodes.iter().find(|(k, _)| *k != &leader)
            .map(|(id, addr)| (*id, addr.clone())).expect("Expected to find non-leader node.");
        debug!("Sending client requests to node {}.", &id);
        fut::wrap_stream(iter_ok::<_, ()>(0u64..20_000u64)).and_then(move |elem, act: &mut Self, _| {
            // Bring down leader after some time.
            if elem == 1000 {
                act.network.do_send(ExecuteInRaftRouter(Box::new(move |act, _| act.isolate_node(leader))));
            }
            // Restore old leader after some time.
            if elem == 15_000 {
                act.network.do_send(ExecuteInRaftRouter(Box::new(move |act, _| act.restore_node(leader))));
            }

            let entry = EntryNormal{data: elem.to_string().into_bytes()};
            let payload = Payload::new(vec![entry], ResponseMode::Applied);
            fut::wrap_future(addr.send(payload))
                .timeout(Duration::from_millis(100), MailboxError::Timeout)
                .map_err(|err, _, _| match err {
                    MailboxError::Closed => panic!("Error sending client request to node. {}", err),
                    MailboxError::Timeout => {
                        error!("Client request timed out.");
                        ClientError::Internal
                    }
                })
                .and_then(|res, _, _| match res {
                    Ok(val) => fut::Either::A(fut::ok(val)),
                    Err(err) => {
                        fut::Either::B(fut::wrap_future(Delay::new(Instant::now() + Duration::from_millis(10))).map_err(|_, _, _| ())
                            .then(move |_, _, _| fut::err(err)))
                    }
                })
                .map(|_res, _, _| ())
                .then(move |_, _, _| {
                    debug!("Entry sent for elem: {}", elem);
                    fut::ok(())
                })
        })
        .finish().then(|_, _, _| {
            debug!("Finished sending client requests.");
            fut::ok(())
        })
    }
}
