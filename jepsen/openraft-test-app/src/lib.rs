#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use openraft::Config;
use openraft::NodeInfo as Node;
use openraft::SnapshotPolicy;
use openraft::storage::RaftLogStorage;

use crate::app::App;

pub mod app;
pub mod http_api;

// Share the RocksDB example's checkpoint transport with Jepsen's type configuration.
#[path = "../../../examples/raft-kv-rocksdb/src/network/mod.rs"]
pub mod network;

pub type NodeId = String;
pub type SnapshotData = sm_rocks::RocksSnapshotData;

openraft::declare_raft_types!(
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        NodeId = NodeId,
        Node = Node,
);

pub type LogStore = log_rocks::RocksLogStore<TypeConfig>;
pub type StateMachineStore = sm_rocks::RocksStateMachine<TypeConfig>;
pub type Raft = openraft::Raft<TypeConfig, StateMachineStore>;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Opt {
    #[clap(long)]
    id: String,

    #[clap(long)]
    api_addr: String,

    #[clap(long)]
    raft_addr: String,

    #[clap(long, value_name = "PATH")]
    data_dir: Option<PathBuf>,

    #[clap(long)]
    snapshot_threshold: Option<NonZeroU64>,

    #[clap(long)]
    enable_pre_vote: bool,

    #[clap(long)]
    disable_leader_restore: bool,
}

#[path = "../../../examples/utils/declare_types.rs"]
pub mod typ;

pub async fn start_raft_node(options: Opt) -> std::io::Result<()> {
    let Opt {
        id: node_id,
        api_addr,
        raft_addr,
        data_dir,
        snapshot_threshold,
        enable_pre_vote,
        disable_leader_restore,
    } = options;
    let dir = data_dir.unwrap_or_else(|| PathBuf::from(format!("{api_addr}.db")));

    // Create a configuration for the raft instance.
    let mut config = Config {
        heartbeat_interval: 50,
        election_timeout_min: 299,
        // A snapshot transfers a whole RocksDB checkpoint in one request.
        install_snapshot_timeout: 10_000,
        // 700 ms exceeds the 600 ms minimum: leader lease + max election timeout.
        quorum_loss_probe_interval: Some(700),
        enable_pre_vote: Some(enable_pre_vote),
        enable_leader_restore: disable_leader_restore.then_some(false),
        ..Default::default()
    };

    if let Some(threshold) = snapshot_threshold {
        config.snapshot_policy = SnapshotPolicy::LogsSinceLast(threshold.get());
    }

    let config = Arc::new(config.validate().unwrap());

    let mut log_store = LogStore::open(dir.join("log"))?;
    let state_machine_dir = dir.join("sm");
    let state_machine_store = StateMachineStore::open(&state_machine_dir)?;
    let is_restart = log_store.get_log_state().await?.last_log_id.is_some();

    let network = network::NetworkFactory::new();

    // Create a local raft instance.
    let raft = openraft::Raft::new(node_id.clone(), config.clone(), network, log_store, state_machine_store)
        .await
        .unwrap();

    let app = Arc::new(App {
        id: node_id,
        api_addr: api_addr.clone(),
        raft_addr: raft_addr.clone(),
        raft,
        data: (),
    });

    let raft_server = network::Server::new(app.raft.clone(), state_machine_dir).run(raft_addr);
    let app_server = async move {
        if is_restart {
            // Jepsen bounds final cluster recovery externally. A local
            // deadline would terminate both servers while a restart is still
            // able to catch up.
            app.raft.wait_for_recovery(None).await.map_err(|e| std::io::Error::other(e.to_string()))?;
        }

        app_http::Server::new(app)
            .add_openraft_routes()
            .post("/append-membership", http_api::append_membership)
            .post("/read", http_api::read)
            .post("/linearizable_read", http_api::linearizable_read)
            .post("/transfer-leader", http_api::transfer_leader)
            .run(api_addr)
            .await
    };

    tokio::try_join!(raft_server, app_server)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use clap::Parser;

    use super::Opt;

    #[test]
    fn snapshot_threshold_must_be_positive() {
        let parse = |threshold| {
            Opt::try_parse_from([
                "openraft-jepsen-app",
                "--id",
                "n1",
                "--api-addr",
                "n1:21001",
                "--raft-addr",
                "n1:22001",
                "--snapshot-threshold",
                threshold,
            ])
        };

        let options = parse("100").unwrap();
        assert_eq!(NonZeroU64::new(100), options.snapshot_threshold);
        assert!(parse("0").is_err());
    }
}
