//! Assert that a single node cluster remains idle.
//!
//! TODO: a few items needed to get a proper test setup.
//! - [ ] finish up snapshot bits on memory storage.
//! - [ ] implement a reusable RaftNetwork implementation (RaftRouter) as a fixture which can be used
//! throughout tests. Should probably incorporate a mechanism which will allow message delivery
//! failure to be induced for testing and the like. Perhaps a simple filter mechanism which will
//! allow for filtering messages inbound to to a specific node and filtering messages outbound
//! from a specific node.
//! - [ ] implement test controllers per test case.
//!   - will be spawned like any other task.
//!   - will run alongside the Raft nodes.
//!   - will define the expectations of the test.
//!   - will panic if specific assertions are not met, just like regular tests.

mod fixtures;

use std::time::Duration;

use actix::prelude::*;
use actix_raft::{
    NodeId, Raft,
    config::Config,
    memory_storage::{MemoryStorage},
    metrics::{RaftMetrics, State},
    storage::RaftStorage,
};
use env_logger;
use tempfile::{tempdir_in, TempDir};

use fixtures::{
    MemRaft,
    router::{AssertAgainstMetrics, RaftRouter, Register},
};

struct TestController {
    network: Addr<RaftRouter>,
    #[allow(dead_code)]
    node0: Addr<MemRaft>,
    #[allow(dead_code)]
    node1: Addr<MemRaft>,
    #[allow(dead_code)]
    node2: Addr<MemRaft>,
}

impl Actor for TestController {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.run_later(std::time::Duration::from_secs(5), |act, _| {
            act.network.do_send(AssertAgainstMetrics(Box::new(|metrics| {
                let node0: &RaftMetrics = metrics.get(&0).unwrap();
                let node1: &RaftMetrics = metrics.get(&1).unwrap();
                let node2: &RaftMetrics = metrics.get(&2).unwrap();
                let data = vec![node0, node1, node2];

                let leader = data.iter().find(|e| &e.state == &State::Leader);
                assert!(leader.is_some(), "Expect leader to exist."); // Assert that we have a leader.
                let leader_id = leader.unwrap().id;
                assert!(data.iter().all(|e| e.current_leader == Some(leader_id)), "Expect all nodes have the same leader.");
                let term = data.first().unwrap().current_term;
                assert!(data.iter().all(|e| e.current_term == term), "Expect all nodes to be at the same term.");
                assert!(data.iter().all(|e| e.last_log_index == 0), "Expect all nodes have last log index '0'.");
                assert!(data.iter().all(|e| e.last_applied == 0), "Expect all nodes have last applied '0'.");
            })));
            System::current().stop();
        });
    }
}

fn new_raft_node(id: NodeId, network: Addr<RaftRouter>, members: Vec<NodeId>) -> (Addr<MemRaft>, TempDir) {
    let temp_dir = tempdir_in("/tmp").unwrap();
    let snapshot_dir = temp_dir.path().to_string_lossy().to_string();
    let config = Config::build(snapshot_dir.clone()).metrics_rate(Duration::from_secs(1)).validate().unwrap();

    let memstore = MemoryStorage::new(members, snapshot_dir);
    let storage = memstore.start();

    let node0 = Raft::new(id, config, network.clone(), storage, network.recipient()).start();
    (node0, temp_dir)
}

#[test]
fn three_node_cluster_should_immediately_elect_leader() {
    env_logger::init();
    let sys = System::builder().stop_on_panic(true).name("test").build();

    let net = RaftRouter::new();
    let network = net.start();
    let members = vec![0, 1, 2];
    let (node0, _f0) = new_raft_node(0, network.clone(), members.clone());
    network.do_send(Register{id: 0, addr: node0.clone()});
    let (node1, _f1) = new_raft_node(1, network.clone(), members.clone());
    network.do_send(Register{id: 1, addr: node1.clone()});
    let (node2, _f2) = new_raft_node(2, network.clone(), members.clone());
    network.do_send(Register{id: 2, addr: node2.clone()});

    let _test = TestController{network: network.clone(), node0, node1, node2}.start();
    let _ = sys.run();
}
