#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use openraft::Config;
use openraft::NodeInfo as Node;
use openraft::SnapshotPolicy;
use openraft::storage::RaftLogStorage;

use crate::app::App;
use crate::store::new_storage;

pub mod app;
pub mod http_api;
pub mod store;

pub type NodeId = String;
pub type SnapshotData = std::io::Cursor<Vec<u8>>;

openraft::declare_raft_types!(
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        NodeId = NodeId,
        Node = Node,
);

pub type LogStore = log_rocks::RocksLogStore<TypeConfig>;
pub type StateMachineStore = store::StateMachineStore;
pub type Raft = openraft::Raft<TypeConfig, StateMachineStore>;

// Jepsen bounds final cluster recovery externally. A local deadline would
// terminate both servers while a restart is still able to catch up.
const RESTART_RECOVERY_TIMEOUT: Option<Duration> = None;

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
    } = options;
    let dir = data_dir.unwrap_or_else(|| PathBuf::from(format!("{api_addr}.db")));

    // Create a configuration for the raft instance.
    let mut config = Config {
        heartbeat_interval: 50,
        election_timeout_min: 299,
        ..Default::default()
    };

    if let Some(threshold) = snapshot_threshold {
        config.snapshot_policy = SnapshotPolicy::LogsSinceLast(threshold.get());
    }

    let config = Arc::new(config.validate().unwrap());

    let (mut log_store, state_machine_store) = new_storage(&dir).await;
    let is_restart = log_store.get_log_state().await?.last_log_id.is_some();

    let kvs = state_machine_store.data.kvs.clone();

    let network = network_v2_http::NetworkFactory::new();

    // Create a local raft instance.
    let raft = openraft::Raft::new(node_id.clone(), config.clone(), network, log_store, state_machine_store)
        .await
        .unwrap();

    let app = Arc::new(App {
        id: node_id,
        api_addr: api_addr.clone(),
        raft_addr: raft_addr.clone(),
        raft,
        data: kvs,
    });

    let raft_server = network_v2_http::Server::new(app.raft.clone()).run(raft_addr);
    let app_server = async move {
        if is_restart {
            app.raft
                .wait_for_recovery(RESTART_RECOVERY_TIMEOUT)
                .await
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }

        app_http::Server::new(app)
            .add_openraft_routes()
            .post("/read", http_api::read)
            .post("/linearizable_read", http_api::linearizable_read)
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
    use super::RESTART_RECOVERY_TIMEOUT;

    #[test]
    fn restart_recovery_has_no_local_deadline() {
        assert!(RESTART_RECOVERY_TIMEOUT.is_none());
    }

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
