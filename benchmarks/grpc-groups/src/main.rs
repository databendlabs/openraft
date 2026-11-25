//! Measure Raft groups whose members run in separate processes and talk over gRPC, in steady
//! operation and through the crash, the hang or the partition of a node. The README describes the
//! scenarios and the fields that a run prints.

mod admin_service;
mod clock;
mod driver;
mod duration_histogram;
mod failover_summary;
mod group_network;
mod node;
mod node_process;
mod partition;
mod phase;
mod raft_config;
mod raft_service;
mod ready;
mod rejoin_summary;
mod rejoins;
mod router;
mod router_stats;
mod scenario;
mod takeovers;
mod window;
mod window_summary;
mod write_stats;
mod writer;

#[path = "../../idle-groups/src/process_usage.rs"]
mod process_usage;

/// The messages and services of `proto/grpc_groups.proto`.
mod protobuf {
    tonic::include_proto!("grpcgroups");
}

use clap::Parser;
use tracing_subscriber::EnvFilter;

pub(crate) type C = raft_kv_memstore_grpc::TypeConfig;
pub(crate) type Raft = raft_kv_memstore_grpc::Raft;

#[derive(Debug, Parser)]
#[command(about = "Measure Raft groups whose members run in separate processes and talk over gRPC")]
enum Command {
    /// Start the node processes, run a scenario, and print one line of fields per phase.
    Run(driver::RunArgs),

    /// Run one node: `run` starts these processes itself.
    Node(node::NodeArgs),
}

fn main() -> anyhow::Result<()> {
    // `RUST_LOG` selects the logs, which go to stderr, so that stdout holds only the results.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    match Command::parse() {
        Command::Run(args) => driver::run(args),
        Command::Node(args) => node::run(args),
    }
}
