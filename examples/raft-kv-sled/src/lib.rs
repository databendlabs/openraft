
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

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub ExampleTypeConfig: D = ExampleRequest, R = ExampleResponse, NodeId = ExampleNodeId
);

pub type ExampleRaft = Raft<ExampleTypeConfig, ExampleNetwork, Arc<ExampleStore>>;
type Server = tide::Server<Arc<ExampleApp>>;
pub async fn start_example_raft_node(
    node_id: ExampleNodeId,
    dir: &str,
    http_addr: String,
    rcp_addr: String,
) -> std::io::Result<()>
{
    // Create a configuration for the raft instance.
    let config = Arc::new(Config::default().validate().unwrap());

    // Create a instance of where the Raft data will be stored.
    let db_dir = std::path::Path::new(dir);
    if !db_dir.exists() {
        std::fs::create_dir_all(db_dir).expect(&format!("could not create: {:?}", db_dir.to_str()))
    }

    let db: sled::Db = sled::open(db_dir)
        .expect(&format!("could not open: {:?}", db_dir.to_str()));
    let store = ExampleStore::new(db).await;

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
