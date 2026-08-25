//! A key-value application with a WAL-backed Raft log and an in-memory state machine.

#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::io;
use std::sync::Arc;

use openraft::Config;
use openraft::NodeInfo as Node;
use openraft::SnapshotPolicy;

use crate::app::App;

pub mod app;
pub mod http_api;

pub type NodeId = u64;
pub type SnapshotData = io::Cursor<Vec<u8>>;

openraft::declare_raft_types!(
    /// Declare the type configuration for the example key-value store.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = Node,
);

pub type LogStore = log_wal::WalLogStore<TypeConfig>;
pub type StateMachineStore = sm_mem::StateMachineStore<TypeConfig>;
pub type Raft = openraft::Raft<TypeConfig, StateMachineStore>;

#[path = "../../utils/declare_types.rs"]
pub mod typ;

pub async fn start_example_raft_node(
    node_id: NodeId,
    data_dir: String,
    api_addr: String,
    raft_addr: String,
) -> io::Result<()> {
    let app = new_app(node_id, data_dir, api_addr, raft_addr).await?;
    let api_addr = app.api_addr.clone();
    let raft_addr = app.raft_addr.clone();

    let raft_server = network_v2_http::Server::new(app.raft.clone()).run(raft_addr);
    let app_server = new_http_server(app).run(api_addr);

    tokio::try_join!(raft_server, app_server)?;
    Ok(())
}

async fn new_app(node_id: NodeId, data_dir: String, api_addr: String, raft_addr: String) -> io::Result<Arc<App>> {
    let config = example_config();
    let (raft, state_machine_store) = new_raft_node(node_id, data_dir, config).await?;

    Ok(Arc::new(App {
        id: node_id,
        api_addr,
        raft_addr,
        raft,
        data: state_machine_store,
    }))
}

/// The timing and storage settings this example runs with.
pub fn example_config() -> Config {
    Config {
        heartbeat_interval: 500,
        election_timeout_min: 1500,
        election_timeout_max: 3000,
        // `sm-mem` loses snapshots on restart, so every WAL entry must remain
        // available for rebuilding the state machine.
        snapshot_policy: SnapshotPolicy::Never,
        max_in_snapshot_log_to_keep: u64::MAX,
        ..Default::default()
    }
}

async fn new_raft_node(node_id: NodeId, data_dir: String, config: Config) -> io::Result<(Raft, StateMachineStore)> {
    let config = config.validate().map_err(io::Error::other)?;
    let config = Arc::new(config);

    let log_store = LogStore::open(data_dir)?;
    let state_machine_store = StateMachineStore::default();
    let network = network_v2_http::NetworkFactory::new();

    let raft = openraft::Raft::new(node_id, config, network, log_store, state_machine_store.clone())
        .await
        .map_err(io::Error::other)?;

    Ok((raft, state_machine_store))
}

fn new_http_server(app: Arc<App>) -> app_http::Server<App> {
    app_http::Server::new(app)
        .add_openraft_routes()
        .post("/read", http_api::read)
        .post("/linearizable_read", http_api::linearizable_read)
        .post("/follower_read", http_api::follower_read)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use openraft::StorageHelper;
    use openraft::entry::RaftEntry;
    use openraft::storage::RaftLogStorage;
    use openraft::storage::RaftLogStorageExt;
    use openraft::storage::RaftStateMachine;
    use openraft::type_config::TypeConfigExt;
    use openraft::type_config::alias::EntryOf;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn rebuilds_state_machine_from_wal_after_restart() {
        TypeConfig::run(async {
            let data_dir_path = TempDir::new().unwrap().keep();
            let data_dir = data_dir_path.display().to_string();
            let mut log_store = LogStore::open(data_dir.clone()).unwrap();

            let first_log_id = openraft::testing::log_id::<TypeConfig>(1, 1, 0);
            let committed_log_id = openraft::testing::log_id::<TypeConfig>(1, 1, 1);
            let first_entry = EntryOf::<TypeConfig>::new_blank(first_log_id);
            let request = types_kv::Request::set("foo", "bar");
            let committed_entry = EntryOf::<TypeConfig>::new_normal(committed_log_id, request);
            log_store.blocking_append([first_entry, committed_entry]).await.unwrap();

            log_store.save_committed(Some(committed_log_id)).await.unwrap();

            // The committed marker does not flush by itself. The next append
            // makes both that marker and this trailing entry durable.
            let trailing_log_id = openraft::testing::log_id::<TypeConfig>(1, 1, 2);
            let trailing_entry = EntryOf::<TypeConfig>::new_blank(trailing_log_id);
            log_store.blocking_append([trailing_entry]).await.unwrap();
            drop(log_store);

            let mut reopened_log_store = LogStore::open(data_dir).unwrap();
            let mut state_machine = StateMachineStore::default();
            StorageHelper::new(&mut reopened_log_store, &mut state_machine)
                .with_id(1)
                .get_initial_state()
                .await
                .unwrap();

            let (last_applied, _) = state_machine.applied_state().await.unwrap();
            assert_eq!(Some(committed_log_id), last_applied);

            let value = state_machine.get("foo").await;
            let expected = Some(types_kv::VersionedValue {
                value: "bar".to_string(),
                version: 1,
            });
            assert_eq!(expected, value);

            drop(reopened_log_store);
            fs::remove_dir_all(data_dir_path).unwrap();
        });
    }
}
