#![allow(dead_code)]
//! Fixtures for testing Raft.

pub mod dev;
pub mod memory_storage;

use std::{
    collections::BTreeMap,
    time::Duration,
};

use actix::prelude::*;
use actix_raft::{
    NodeId, Raft,
    config::{Config, SnapshotPolicy},
    messages::{ClientPayload, ClientError, EntryNormal, ResponseMode},
};
use async_log;
use env_logger;
use futures::sync::oneshot;
use log::{debug};
use tempfile::{tempdir_in, TempDir};

use crate::fixtures::{
    dev::{GetCurrentLeader, MemRaft, RaftRouter},
    memory_storage::{MemoryStorage, MemoryStorageData, MemoryStorageError, MemoryStorageResponse},
};

pub type Payload = ClientPayload<MemoryStorageData, MemoryStorageResponse, MemoryStorageError>;

pub fn setup_logger() {
    let logger = env_logger::Builder::from_default_env().build();
    async_log::Logger::wrap(logger, || 12)
        .start(log::LevelFilter::Trace)
        .expect("Expected to be able to start async logger.");
}

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

/// Send a client request to the Raft cluster, ensuring it is resent on failures.
#[derive(Message)]
pub struct ClientRequest {
    pub payload: u64,
    pub current_leader: Option<NodeId>,
    pub cb: Option<oneshot::Sender<()>>,
}

impl Handler<ClientRequest> for RaftTestController {
    type Result = ();

    fn handle(&mut self, mut msg: ClientRequest, ctx: &mut Self::Context) {
        if let Some(leader) = &msg.current_leader {
            let entry = EntryNormal{data: MemoryStorageData{data: msg.payload.to_string().into_bytes()}};
            let payload = Payload::new(entry, ResponseMode::Applied);
            let node = self.nodes.get(leader).expect("Expected leader to be present it RaftTestController's nodes map.");
            let f = fut::wrap_future(node.send(payload))
                .map_err(|_, _, _| ClientError::Internal).and_then(|res, _, _| fut::result(res))
                .then(move |res, _, ctx: &mut Context<Self>| match res {
                    Ok(_) => {
                        if let Some(tx) = msg.cb {
                            let _ = tx.send(()).expect("Failed to send callback message on test's ClientRequest.");
                        }
                        fut::ok(())
                    },
                    Err(err) => match err {
                        ClientError::Internal => {
                            debug!("TEST: resending client request.");
                            ctx.notify(msg);
                            fut::ok(())
                        }
                        ClientError::Application(err) => {
                            panic!("Unexpected application error from client request: {:?}", err)
                        }
                        ClientError::ForwardToLeader{leader, ..} => {
                            debug!("TEST: received ForwardToLeader error. Updating leader and forwarding.");
                            msg.current_leader = leader;
                            ctx.notify(msg);
                            fut::ok(())
                        }
                    }
                });
            ctx.spawn(f);
        } else {
            debug!("TEST: client request has no known leader. Waiting for 100ms before checking again.");
            ctx.run_later(Duration::from_secs(2), move |act, ctx| {
                let f = fut::wrap_future(act.network.send(GetCurrentLeader))
                    .map_err(|err, _, _| panic!("{}", err))
                    .and_then(|res, _, _| fut::result(res))
                    .then(move |res, _, ctx: &mut Context<Self>| match res {
                        Ok(current_leader) => {
                            msg.current_leader = current_leader;
                            ctx.notify(msg);
                            fut::ok(())
                        }
                        Err(_) => {
                            ctx.notify(msg);
                            fut::ok(())
                        }
                    });
                ctx.spawn(f);
            });
        }
    }
}

/// All objects related to a started Raft node.
pub struct Node {
    pub raft_arb: Arbiter,
    pub storage_arb: Arbiter,
    pub addr: Addr<MemRaft>,
    pub snapshot_dir: TempDir,
    pub storage: Addr<MemoryStorage>,
}

impl Node {
    /// Start building a new node.
    pub fn builder(id: NodeId, network: Addr<RaftRouter>, members: Vec<NodeId>) -> NodeBuilder {
        NodeBuilder{id, network, members, metrics_rate: None, snapshot_policy: None}
    }
}

pub struct NodeBuilder {
    id: NodeId,
    network: Addr<RaftRouter>,
    members: Vec<NodeId>,
    metrics_rate: Option<u64>,
    snapshot_policy: Option<SnapshotPolicy>,
}

impl NodeBuilder {
    /// Build the new node.
    pub fn build(self) -> Node {
        let metrics_rate = self.metrics_rate.unwrap_or(1);
        let snapshot_policy = self.snapshot_policy.unwrap_or(SnapshotPolicy::default());
        let id = self.id;
        let members = self.members;
        let network = self.network;

        let temp_dir = tempdir_in("/tmp").expect("Tempdir to be created without error.");
        let snapshot_dir = temp_dir.path().to_string_lossy().to_string();
        let config = Config::build(snapshot_dir.clone())
            .election_timeout_min(1500).election_timeout_max(2000).heartbeat_interval(150)
            .metrics_rate(Duration::from_secs(metrics_rate))
            .snapshot_policy(snapshot_policy).snapshot_max_chunk_size(10000)
            .validate().expect("Raft config to be created without error.");

        let (storage_arb, raft_arb) = (Arbiter::new(), Arbiter::new());
        let storage = MemoryStorage::start_in_arbiter(&storage_arb, |_| MemoryStorage::new(members, snapshot_dir));
        let storage_addr = storage.clone();
        let addr = Raft::start_in_arbiter(&raft_arb, move |_| {
            Raft::new(id, config, network.clone(), storage.clone(), network.recipient())
        });
        Node{addr, snapshot_dir: temp_dir, storage_arb, raft_arb, storage: storage_addr}
    }

    /// Configure the metrics rate for this node, defaults to 1 second.
    pub fn metrics_rate(mut self, val: u64) -> Self {
        self.metrics_rate = Some(val);
        self
    }

    /// Configure the node's snapshot policy, defaults to `SnapshotPolicy::default()`.
    pub fn snapshot_policy(mut self, val: SnapshotPolicy) -> Self {
        self.snapshot_policy = Some(val);
        self
    }
}

/// Create a new Raft node for testing purposes.
pub fn new_raft_node(id: NodeId, network: Addr<RaftRouter>, members: Vec<NodeId>, metrics_rate: u64) -> Node {
    let temp_dir = tempdir_in("/tmp").expect("Tempdir to be created without error.");
    let snapshot_dir = temp_dir.path().to_string_lossy().to_string();
    let config = Config::build(snapshot_dir.clone())
        .election_timeout_min(1500).election_timeout_max(2000).heartbeat_interval(150)
        .metrics_rate(Duration::from_secs(metrics_rate))
        .snapshot_policy(SnapshotPolicy::Disabled).snapshot_max_chunk_size(10000)
        .validate().expect("Raft config to be created without error.");

    let (storage_arb, raft_arb) = (Arbiter::new(), Arbiter::new());
    let storage = MemoryStorage::start_in_arbiter(&storage_arb, |_| MemoryStorage::new(members, snapshot_dir));
    let storage_addr = storage.clone();
    let addr = Raft::start_in_arbiter(&raft_arb, move |_| {
        Raft::new(id, config, network.clone(), storage.clone(), network.recipient())
    });
    Node{addr, snapshot_dir: temp_dir, storage_arb, raft_arb, storage: storage_addr}
}
