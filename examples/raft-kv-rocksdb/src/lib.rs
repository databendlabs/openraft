#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::fmt::Display;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use async_std::net::TcpListener;
use async_std::task;
use openraft::storage::Adaptor;
use openraft::Config;
use openraft::TokioRuntime;

use crate::app::App;
use crate::network::api;
use crate::network::management;
use crate::network::Network;
use crate::store::Request;
use crate::store::Response;
use crate::store::Store;

pub mod app;
pub mod client;
pub mod network;
pub mod store;

pub type NodeId = u64;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct Node {
    pub rpc_addr: String,
    pub api_addr: String,
}

impl Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Node {{ rpc_addr: {}, api_addr: {} }}", self.rpc_addr, self.api_addr)
    }
}

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig: D = Request, R = Response, NodeId = NodeId, Node = Node,
    Entry = openraft::Entry<TypeConfig>, SnapshotData = Cursor<Vec<u8>>, AsyncRuntime = TokioRuntime
);

pub type LogStore = Adaptor<TypeConfig, Arc<Store>>;
pub type StateMachineStore = Adaptor<TypeConfig, Arc<Store>>;
pub type ExampleRaft = openraft::Raft<TypeConfig, Network, LogStore, StateMachineStore>;

type Server = tide::Server<Arc<App>>;

pub async fn start_example_raft_node<P>(
    node_id: NodeId,
    dir: P,
    http_addr: String,
    rcp_addr: String,
) -> std::io::Result<()>
where
    P: AsRef<Path>,
{
    // Create a configuration for the raft instance.
    let config = Config {
        heartbeat_interval: 250,
        election_timeout_min: 299,
        ..Default::default()
    };

    let config = Arc::new(config.validate().unwrap());

    // Create a instance of where the Raft data will be stored.
    let store = Store::new(&dir).await;

    let (log_store, state_machine) = Adaptor::new(store.clone());

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = Network {};

    // Create a local raft instance.
    let raft = openraft::Raft::new(node_id, config.clone(), network, log_store, state_machine).await.unwrap();

    let app = Arc::new(App {
        id: node_id,
        api_addr: http_addr.clone(),
        rcp_addr: rcp_addr.clone(),
        raft,
        store,
        config,
    });

    let echo_service = Arc::new(network::raft::Raft::new(app.clone()));

    let server = toy_rpc::Server::builder().register(echo_service).build();

    let listener = TcpListener::bind(rcp_addr).await.unwrap();
    let handle = task::spawn(async move {
        server.accept_websocket(listener).await.unwrap();
    });

    // Create an application that will store all the instances created above, this will
    // be later used on the actix-web services.
    let mut app: Server = tide::Server::with_state(app);

    management::rest(&mut app);
    api::rest(&mut app);

    app.listen(http_addr).await?;
    handle.await;
    Ok(())
}
