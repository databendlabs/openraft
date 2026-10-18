#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::fmt::Debug;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use openraft::Config;
use openraft::NodeInfo as Node;
use openraft::SnapshotPolicy;
use openraft::async_runtime::WatchReceiver;
use openraft::storage::RaftLogStorage;

use crate::app::App;
use crate::app::AppData;
use crate::store::new_storage;

pub mod app;
pub mod http_api;
pub mod request;
pub mod store;

pub type NodeId = String;

/// Printed once per snapshot installed from a leader, for Jepsen to count installs per node.
pub const SNAPSHOT_INSTALLED_MARKER: &str = "OPENRAFT_JEPSEN_SNAPSHOT_INSTALLED";

/// Printed when a metric that never decreases within one process decreases.
pub const INVARIANT_MARKER: &str = "OPENRAFT_JEPSEN_INVARIANT_VIOLATION";
pub type SnapshotData = std::io::Cursor<Vec<u8>>;

openraft::declare_raft_types!(
    pub TypeConfig:
        D = request::Request,
        R = types_kv::Response,
        NodeId = NodeId,
        Node = Node,
);

pub type LogStore = log_rocks::RocksLogStore<TypeConfig>;
pub type StateMachineStore = store::StateMachineStore;
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

    /// Applied logs to keep after a snapshot; 0 purges every log the snapshot covers.
    #[clap(long)]
    max_in_snapshot_log_to_keep: Option<u64>,

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
        max_in_snapshot_log_to_keep,
        enable_pre_vote,
        disable_leader_restore,
    } = options;
    let dir = data_dir.unwrap_or_else(|| PathBuf::from(format!("{api_addr}.db")));

    // Create a configuration for the raft instance.
    let mut config = Config {
        heartbeat_interval: 50,
        election_timeout_min: 299,
        // 700 ms exceeds the 600 ms minimum: leader lease + max election timeout.
        quorum_loss_probe_interval: Some(700),
        enable_pre_vote: Some(enable_pre_vote),
        enable_leader_restore: disable_leader_restore.then_some(false),
        ..Default::default()
    };

    if let Some(threshold) = snapshot_threshold {
        config.snapshot_policy = SnapshotPolicy::LogsSinceLast(threshold.get());
    }

    if let Some(keep) = max_in_snapshot_log_to_keep {
        config.max_in_snapshot_log_to_keep = keep;
    }

    let config = Arc::new(config.validate().unwrap());

    let (mut log_store, state_machine_store) = new_storage(&dir).await;
    let is_restart = log_store.get_log_state().await?.last_log_id.is_some();

    let data = AppData {
        state: state_machine_store.state.clone(),
        log_store: log_store.clone(),
    };

    let network = network_v2_http::NetworkFactory::new();

    // Create a local raft instance.
    let raft = openraft::Raft::new(node_id.clone(), config.clone(), network, log_store, state_machine_store)
        .await
        .unwrap();

    tokio::spawn(watch_monotonic_metrics(raft.clone()));

    let app = Arc::new(App {
        id: node_id,
        api_addr: api_addr.clone(),
        raft_addr: raft_addr.clone(),
        raft,
        data,
    });

    let raft_server = network_v2_http::Server::new(app.raft.clone()).run(raft_addr);
    let app_server = async move {
        if is_restart {
            // Jepsen bounds final cluster recovery externally. A local
            // deadline would terminate both servers while a restart is still
            // able to catch up.
            app.raft.wait_for_recovery(None).await.map_err(|e| std::io::Error::other(e.to_string()))?;
        }

        app_http::Server::new(app)
            .add_openraft_routes()
            .get("/consistency", http_api::consistency)
            .post("/linearizable_read_list", http_api::linearizable_read_list)
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

/// Reports every decrease of a metric that never decreases within one process.
///
/// Jepsen samples `/metrics` too sparsely to see a regression that recovers before the next
/// sample, while this wakes on every change; the watch channel still keeps only the latest
/// value. A restarted process starts over, so a process only compares its own values.
async fn watch_monotonic_metrics(raft: Raft) {
    let mut metrics = raft.metrics();
    let mut prev = metrics.borrow_watched().clone();

    while metrics.changed().await.is_ok() {
        let cur = metrics.borrow_watched().clone();

        report_decrease("current_term", &prev.current_term, &cur.current_term);
        report_decrease("local_committed", &prev.local_committed, &cur.local_committed);
        report_decrease("last_applied", &prev.last_applied, &cur.last_applied);
        report_decrease("snapshot", &prev.snapshot, &cur.snapshot);
        report_decrease("purged", &prev.purged, &cur.purged);

        prev = cur;
    }
}

fn report_decrease<T>(metric: &str, prev: &T, cur: &T)
where T: PartialOrd + Debug {
    if cur < prev {
        eprintln!("{INVARIANT_MARKER} {metric} decreased from {prev:?} to {cur:?}");
    }
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

    #[test]
    fn max_in_snapshot_log_to_keep_accepts_zero() {
        let options = Opt::try_parse_from([
            "openraft-jepsen-app",
            "--id",
            "n1",
            "--api-addr",
            "n1:21001",
            "--raft-addr",
            "n1:22001",
            "--max-in-snapshot-log-to-keep",
            "0",
        ])
        .unwrap();
        assert_eq!(Some(0), options.max_in_snapshot_log_to_keep);
    }
}
