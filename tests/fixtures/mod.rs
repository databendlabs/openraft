//! Fixtures for testing Raft.

use std::{
    collections::BTreeMap,
    time::Duration,
};

use actix::prelude::*;
use actix_raft::{
    NodeId, Raft,
    config::{Config, SnapshotPolicy},
    dev::{MemRaft, RaftRouter},
    memory_storage::{MemoryStorage},
    storage::RaftStorage,
};
use tempfile::{tempdir_in, TempDir};

pub struct RaftTestController {
    pub network: Addr<RaftRouter>,
    pub nodes: BTreeMap<NodeId, Addr<MemRaft>>,
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
        let initial_delay = self.initial_test_delay.take().expect("Test misconfigured. Missing `initial_test_delay`. Use `start_with_test`.");
        let test_func = self.test_func.take().expect("Test misconfigured. Missing `test_func`. Use `start_with_test`.");
        ctx.run_later(initial_delay, test_func);
    }
}

/// Create a new Raft node for testing purposes.
pub fn new_raft_node(id: NodeId, network: Addr<RaftRouter>, members: Vec<NodeId>, metrics_rate: u64) -> Node {
    let temp_dir = tempdir_in("/tmp").expect("Tempdir to be created without error.");
    let snapshot_dir = temp_dir.path().to_string_lossy().to_string();
    let config = Config::build(snapshot_dir.clone())
        .metrics_rate(Duration::from_secs(metrics_rate))
        .snapshot_policy(SnapshotPolicy::Disabled)
        .validate().expect("Raft config to be created without error.");

    let arb = Arbiter::new();
    let storage = MemoryStorage::start_in_arbiter(&arb, |_| MemoryStorage::new(members, snapshot_dir));
    let storage_addr = storage.clone();
    let addr = Raft::start_in_arbiter(&arb, move |_| {
        Raft::new(id, config, network.clone(), storage.clone(), network.recipient())
    });
    Node{addr, snapshot_dir: temp_dir, arb, storage: storage_addr}
}

/// All objects related to a started Raft node.
pub struct Node {
    pub addr: Addr<MemRaft>,
    pub snapshot_dir: TempDir,
    pub arb: Arbiter,
    pub storage: Addr<MemoryStorage>,
}
