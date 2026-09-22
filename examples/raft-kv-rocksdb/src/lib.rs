#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use openraft::Config;
use openraft::NodeInfo as Node;

use crate::app::App;

pub mod app;
pub mod http_api;
pub mod network;

pub type NodeId = u64;
pub type SnapshotData = sm_rocks::RocksSnapshotData;

openraft::declare_raft_types!(
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = Node,
);

pub type LogStore = log_rocks::RocksLogStore<TypeConfig>;
pub type StateMachineStore = sm_rocks::RocksStateMachine<TypeConfig>;
pub type Raft = openraft::Raft<TypeConfig, StateMachineStore>;

#[path = "../../utils/declare_types.rs"]
pub mod typ;

pub async fn start_example_raft_node<P>(
    node_id: NodeId,
    dir: P,
    api_addr: String,
    raft_addr: String,
) -> std::io::Result<()>
where
    P: AsRef<Path>,
{
    start_example_raft_node_with_config(node_id, dir, api_addr, raft_addr, example_config()).await
}

/// Same as [`start_example_raft_node`] but with a caller-provided [`Config`].
///
/// Tests use it to reach behaviors [`example_config()`] never triggers, such as snapshot
/// replication to a learner.
pub async fn start_example_raft_node_with_config<P>(
    node_id: NodeId,
    dir: P,
    api_addr: String,
    raft_addr: String,
    config: Config,
) -> std::io::Result<()>
where
    P: AsRef<Path>,
{
    let node = RaftNode::new(node_id, dir, api_addr, raft_addr, config).await?;
    node.run().await
}

/// One node of the example: the Raft instance behind the HTTP application, assembled but not
/// yet serving.
///
/// The binary assembles and runs it in one go. A test drives the node through `app.raft` and
/// stops it by dropping the [`RaftNode::run`] future.
pub struct RaftNode {
    pub app: Arc<App>,

    /// The state machine's directory, where a received checkpoint becomes a new generation.
    state_machine_dir: PathBuf,
}

impl RaftNode {
    /// Open the two RocksDB databases under `dir`, create the Raft instance, and wrap it in the
    /// HTTP application.
    pub async fn new<P>(
        node_id: NodeId,
        dir: P,
        api_addr: String,
        raft_addr: String,
        config: Config,
    ) -> std::io::Result<Self>
    where
        P: AsRef<Path>,
    {
        let config = config.validate().map_err(std::io::Error::other)?;
        let config = Arc::new(config);

        // The log store and the state machine are separate RocksDB databases.
        let log_store = LogStore::open(dir.as_ref().join("log"))?;
        let state_machine_dir = dir.as_ref().join("sm");
        let state_machine_store = StateMachineStore::open(&state_machine_dir)?;

        let network = network::NetworkFactory::new();

        // Create a local raft instance.
        let raft = openraft::Raft::new(node_id, config, network, log_store, state_machine_store)
            .await
            .map_err(std::io::Error::other)?;

        let app = Arc::new(App {
            id: node_id,
            api_addr,
            raft_addr,
            raft,
            data: (),
        });

        Ok(Self { app, state_machine_dir })
    }

    /// Serve the Raft RPCs on `raft_addr` and the application API on `api_addr` until one of the
    /// two servers fails.
    pub async fn run(self) -> std::io::Result<()> {
        let api_addr = self.app.api_addr.clone();
        let raft_addr = self.app.raft_addr.clone();

        let raft_server = network::Server::new(self.app.raft.clone(), self.state_machine_dir).run(raft_addr);
        let app_server = app_http::Server::new(self.app)
            .add_openraft_routes()
            .post("/read", http_api::read)
            .post("/linearizable_read", http_api::linearizable_read)
            .run(api_addr);

        tokio::try_join!(raft_server, app_server)?;
        Ok(())
    }
}

/// The timing settings this example runs with.
///
/// A snapshot is a whole RocksDB checkpoint sent in one request, so its timeout is far above
/// the default made for small in-memory snapshots.
pub fn example_config() -> Config {
    Config {
        heartbeat_interval: 50,
        election_timeout_min: 299,
        install_snapshot_timeout: 10_000,
        enable_pre_vote: Some(true),
        ..Default::default()
    }
}
