//! Test clustering behavior.

mod fixtures;

use std::{
    collections::BTreeMap,
    time::Duration,
};

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

struct RaftTestController {
    network: Addr<RaftRouter>,
    nodes: BTreeMap<NodeId, Addr<MemRaft>>,
    initial_test_delay: Option<Duration>,
    test_func: Option<Box<dyn FnOnce(&mut RaftTestController, &mut Context<RaftTestController>) + 'static>>,
}

impl RaftTestController {
    /// Create a new instance.
    pub fn new(network: Addr<RaftRouter>) -> Self {
        Self{network, nodes: Default::default(), initial_test_delay: None, test_func: None}
    }

    /// Register a node on the test controller.
    pub fn register(&mut self, id: NodeId, node: Addr<MemRaft>) -> &mut Self {
        self.nodes.insert(id, node);
        self
    }

    /// Start this test controller with the given delay and test function.
    pub fn start_with_test(mut self, delay: u64, test: Box<dyn FnOnce(&mut RaftTestController, &mut Context<RaftTestController>) + 'static>) -> Addr<Self> {
        self.initial_test_delay = Some(Duration::from_secs(delay));
        self.test_func = Some(test);
        self.start()
    }
}

impl Actor for RaftTestController {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        if self.initial_test_delay.is_none() || self.test_func.is_none() {
            panic!("Test is misconfigured. Missing initial test delay or test function. Use `start_with_test`.");
        }
        ctx.run_later(self.initial_test_delay.take().unwrap(), self.test_func.take().unwrap());
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

    // Setup test dependencies.
    let net = RaftRouter::new();
    let network = net.start();
    let members = vec![0, 1, 2];
    let (node0, _f0) = new_raft_node(0, network.clone(), members.clone());
    network.do_send(Register{id: 0, addr: node0.clone()});
    let (node1, _f1) = new_raft_node(1, network.clone(), members.clone());
    network.do_send(Register{id: 1, addr: node1.clone()});
    let (node2, _f2) = new_raft_node(2, network.clone(), members.clone());
    network.do_send(Register{id: 2, addr: node2.clone()});

    // Setup test controller and actions.
    let mut ctl = RaftTestController::new(network);
    ctl.register(0, node0).register(1, node1).register(2, node2);
    ctl.start_with_test(5,  Box::new(|act, _| {
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
    }));
    let _ = sys.run();
}
