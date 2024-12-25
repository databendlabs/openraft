#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::sync::Arc;

use bytes::Buf;
use opendal::Buffer;
use opendal::Operator;
use openraft::Config;

use crate::app::App;
use crate::router::Router;
use crate::store::Request;
use crate::store::Response;

pub mod router;

pub mod api;
pub mod app;
pub mod network;
pub mod store;

pub type NodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig:
        D = Request,
        R = Response,
        // In this example, snapshot is a path pointing to a file stored in shared storage.
        SnapshotData = String
);

pub type LogStore = store::LogStore;
pub type StateMachineStore = store::StateMachineStore;

#[path = "../../utils/declare_types.rs"]
pub mod typ;

pub fn encode<T: serde::Serialize>(t: T) -> String {
    serde_json::to_string(&t).unwrap()
}

pub fn decode<T: serde::de::DeserializeOwned>(s: &str) -> T {
    serde_json::from_str(s).unwrap()
}

pub fn decode_buffer<T: serde::de::DeserializeOwned>(b: Buffer) -> T {
    serde_json::from_reader(b.reader()).unwrap()
}

pub async fn new_raft(node_id: NodeId, router: Router, op: Operator) -> (typ::Raft, App) {
    // Create a configuration for the raft instance.
    let config = Config {
        heartbeat_interval: 500,
        election_timeout_min: 1500,
        election_timeout_max: 3000,
        // Once snapshot is built, delete the logs at once.
        // So that all further replication will be based on the snapshot.
        max_in_snapshot_log_to_keep: 0,
        ..Default::default()
    };

    let config = Arc::new(config.validate().unwrap());

    // Create an instance of where the Raft logs will be stored.
    let log_store = LogStore::default();

    // Create a instance of where the state machine data will be stored.
    let state_machine_store = Arc::new(StateMachineStore::new(op.clone()));

    // Create a local raft instance.
    let raft = openraft::Raft::new(node_id, config, router.clone(), log_store, state_machine_store.clone())
        .await
        .unwrap();

    let app = App::new(node_id, raft.clone(), router, state_machine_store);

    (raft, app)
}
