//! Separate-process benchmark node, reusing example stores and transport adapters.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

mod benchmark_metrics;
mod faults;
mod http_server;
mod network;
#[cfg(feature = "rocks")]
mod rocks;
mod runtime_stats;
mod slow_apply;
mod wal;

#[derive(Clone, Copy, clap::ValueEnum)]
enum Backend {
    Wal,
    Rocks,
}

#[derive(Parser)]
struct Options {
    #[arg(long)]
    id: u64,
    #[arg(long)]
    api_addr: String,
    #[arg(long)]
    raft_addr: String,
    #[arg(long)]
    benchmark_control_addr: String,
    #[arg(long)]
    data_dir: PathBuf,
    #[arg(long, value_enum, default_value = "wal")]
    backend: Backend,
    #[arg(long)]
    snapshot_logs: Option<u64>,
    #[arg(long)]
    snapshot_keep: Option<u64>,
    /// Delay every `apply()` call of the WAL backend's state machine by this many microseconds.
    #[arg(long, default_value_t = 0)]
    apply_delay_us: u64,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let options = Options::parse();
    validate(&options)?;
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    benchmark_metrics::enable();
    let faults = faults::Faults::new();
    let node = run(&options, faults.clone());
    let control = faults.serve(options.benchmark_control_addr.clone());
    tokio::try_join!(node, control)?;
    Ok(())
}

fn validate(options: &Options) -> std::io::Result<()> {
    if options.snapshot_logs == Some(0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "snapshot-logs must be positive",
        ));
    }
    if matches!(options.backend, Backend::Wal) && (options.snapshot_logs.is_some() || options.snapshot_keep.is_some()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "WAL snapshots are disabled",
        ));
    }
    if matches!(options.backend, Backend::Rocks) && options.apply_delay_us > 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "apply-delay-us requires the WAL backend",
        ));
    }
    Ok(())
}

async fn run(options: &Options, faults: Arc<faults::Faults>) -> std::io::Result<()> {
    match options.backend {
        Backend::Wal => wal::run(options, faults).await,
        Backend::Rocks => {
            #[cfg(feature = "rocks")]
            {
                rocks::run(options, faults).await
            }
            #[cfg(not(feature = "rocks"))]
            {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "build the durable node with --features rocks",
                ))
            }
        }
    }
}
