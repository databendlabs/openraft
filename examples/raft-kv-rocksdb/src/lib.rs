use std::fmt::Display;
use std::path::Path;
use std::sync::Arc;

use async_std::net::TcpListener;
use async_std::task;
use openraft::Config;
use openraft::Raft;

use crate::app::ExampleApp;
use crate::network::api;
use crate::network::management;
use crate::network::raft_network_impl::ExampleNetwork;
use crate::store::ExampleRequest;
use crate::store::ExampleResponse;
use crate::store::ExampleStore;

pub mod app;
pub mod client;
pub mod network;
pub mod store;

pub type ExampleNodeId = u64;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct ExampleNode {
    pub rpc_addr: String,
    pub api_addr: String,
}

impl Display for ExampleNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ExampleNode {{ rpc_addr: {}, api_addr: {} }}",
            self.rpc_addr, self.api_addr
        )
    }
}

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub ExampleTypeConfig: D = ExampleRequest, R = ExampleResponse, NodeId = ExampleNodeId, Node = ExampleNode
);

pub type ExampleRaft = Raft<ExampleTypeConfig, ExampleNetwork, Arc<ExampleStore>>;
type Server = tide::Server<Arc<ExampleApp>>;
pub async fn start_example_raft_node<P>(
    node_id: ExampleNodeId,
    dir: P,
    http_addr: String,
    rcp_addr: String,
) -> std::io::Result<()>
where
    P: AsRef<Path>,
{
    // Create a configuration for the raft instance.
    let mut config = Config::default();
    config.heartbeat_interval = 250;
    config.election_timeout_min = 299;

    let config = Arc::new(config.validate().unwrap());

    // Create a instance of where the Raft data will be stored.
    let store = ExampleStore::new(&dir).await;

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = ExampleNetwork {};

    // Create a local raft instance.
    let raft = Raft::new(node_id, config.clone(), network, store.clone());

    let app = Arc::new(ExampleApp {
        id: node_id,
        api_addr: http_addr.clone(),
        rcp_addr: rcp_addr.clone(),
        raft,
        store,
        config,
    });

    let echo_service = Arc::new(crate::network::raft::Raft::new(app.clone()));

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
