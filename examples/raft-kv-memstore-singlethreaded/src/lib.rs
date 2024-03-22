#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::io::Cursor;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use openraft::BasicNode;
use openraft::Config;
use openraft::TokioRuntime;

use crate::app::App;
use crate::router::Router;
use crate::store::Request;
use crate::store::Response;

pub mod router;

pub mod api;
pub mod app;
pub mod network;
pub mod store;

// pub type NodeId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(Default)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct NodeId {
    pub id: u64,
    _p: PhantomData<*const ()>,
}

impl NodeId {
    pub fn new(id: u64) -> Self {
        Self { id, _p: PhantomData }
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.id.fmt(f)
    }
}

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig:
        D = Request,
        R = Response,
        NodeId = NodeId,
        Node = BasicNode,
        Entry = openraft::Entry<TypeConfig>,
        SnapshotData = Cursor<Vec<u8>>,
        AsyncRuntime = TokioRuntime
);

pub type LogStore = store::LogStore;
pub type StateMachineStore = store::StateMachineStore;
pub type Raft = openraft::Raft<TypeConfig>;

pub mod typ {

    use crate::TypeConfig;

    pub type RaftError<E = openraft::error::Infallible> = openraft::error::RaftError<TypeConfig, E>;
    pub type RPCError<E = openraft::error::Infallible> = openraft::error::RPCError<TypeConfig, RaftError<E>>;

    pub type RaftMetrics = openraft::RaftMetrics<TypeConfig>;

    pub type ClientWriteError = openraft::error::ClientWriteError<TypeConfig>;
    pub type CheckIsLeaderError = openraft::error::CheckIsLeaderError<TypeConfig>;
    pub type ForwardToLeader = openraft::error::ForwardToLeader<TypeConfig>;
    pub type InitializeError = openraft::error::InitializeError<TypeConfig>;

    pub type ClientWriteResponse = openraft::raft::ClientWriteResponse<TypeConfig>;
}

pub fn encode<T: serde::Serialize>(t: T) -> String {
    serde_json::to_string(&t).unwrap()
}

pub fn decode<T: serde::de::DeserializeOwned>(s: &str) -> T {
    serde_json::from_str(s).unwrap()
}

pub async fn start_raft(node_id: NodeId, router: Router) -> std::io::Result<()> {
    // Create a configuration for the raft instance.
    let config = Config {
        heartbeat_interval: 500,
        election_timeout_min: 1500,
        election_timeout_max: 3000,
        ..Default::default()
    };

    let config = Arc::new(config.validate().unwrap());

    // Create a instance of where the Raft logs will be stored.
    let log_store = Rc::new(LogStore::default());

    // Create a instance of where the state machine data will be stored.
    let state_machine_store = Rc::new(StateMachineStore::default());

    // Create a local raft instance.
    let raft = openraft::Raft::new(node_id, config, router.clone(), log_store, state_machine_store.clone())
        .await
        .unwrap();

    // Create an application that will store all the instances created above, this will
    // later be used on the actix-web services.
    let app = App::new(node_id, raft, router, state_machine_store);

    app.run().await.unwrap();

    tracing::info!("Raft node {} quit", node_id);
    Ok(())
}
