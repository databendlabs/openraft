#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::sync::Arc;

use openraft::Config;
use openraft::NodeInfo as Node;
use openraft_membership::WeightedChange;
use openraft_membership::WeightedMembership;

use crate::app::App;

pub mod app;
pub mod http_api;
pub mod membership_api;
pub mod store;

pub type NodeId = u64;

openraft::declare_raft_types!(
    /// The type config of the example K/V store, with a weighted membership in place of the
    /// default majority.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = Node,
        Membership = WeightedMembership<NodeId, Node>,
        MembershipEdit = WeightedChange<NodeId, Node>,
);

pub type LogStore = store::LogStore<TypeConfig>;
pub type StateMachineStore = store::StateMachineStore<TypeConfig>;
pub type Raft = openraft::Raft<TypeConfig, StateMachineStore>;

pub async fn start_example_raft_node(node_id: NodeId, api_addr: String, raft_addr: String) -> std::io::Result<()> {
    let (raft, state_machine_store) = new_raft_node(node_id, example_config()).await;

    let app = Arc::new(App {
        id: node_id,
        api_addr: api_addr.clone(),
        raft_addr: raft_addr.clone(),
        raft,
        data: state_machine_store,
    });

    let raft_server = network_v2_http::Server::new(app.raft.clone()).run(raft_addr);
    let app_server = new_http_server(app).run(api_addr);

    tokio::try_join!(raft_server, app_server)?;
    Ok(())
}

/// The timing settings this example runs with.
pub fn example_config() -> Config {
    Config {
        heartbeat_interval: 500,
        election_timeout_min: 1500,
        election_timeout_max: 3000,
        enable_pre_vote: Some(true),
        ..Default::default()
    }
}

/// Create the `Raft` instance from its building blocks. The `StateMachineStore` is returned
/// alongside so it can be reused as the application's data handle.
async fn new_raft_node(node_id: NodeId, config: Config) -> (Raft, StateMachineStore) {
    let config = Arc::new(config.validate().unwrap());

    // Create an instance of where the Raft logs will be stored.
    let log_store = LogStore::default();
    // Create an instance of where the Raft data will be stored.
    let state_machine_store = StateMachineStore::default();

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = network_v2_http::NetworkFactory::new();

    // Create a local raft instance.
    let raft = openraft::Raft::new(node_id, config, network, log_store, state_machine_store.clone()).await.unwrap();

    (raft, state_machine_store)
}

/// Build the application HTTP server: the membership routes of this example, the other OpenRaft
/// routes of `app-http`, and this example's reads.
///
/// `app_http::Server::add_openraft_routes()` needs `MembershipEdit: From<ChangeMembers>`, because
/// its `/change-membership` takes only a set of voter IDs. A voter set states no weights, so
/// `WeightedChange` has no such conversion. This server registers the routes one by one instead:
/// `/init` and `/change-membership` from [`membership_api`], which take weights, and the other
/// routes that `add_openraft_routes()` would add.
fn new_http_server(app: Arc<App>) -> app_http::Server<App> {
    app_http::Server::new(app)
        .post("/init", membership_api::init)
        .post("/change-membership", membership_api::change_membership)
        .post("/add-learner", App::add_learner)
        .get("/metrics", App::metrics)
        .get("/get_linearizer", App::get_linearizer)
        .post("/write", App::write)
        .post("/read", http_api::read)
        .post("/linearizable_read", http_api::linearizable_read)
        .post("/follower_read", http_api::follower_read)
}
