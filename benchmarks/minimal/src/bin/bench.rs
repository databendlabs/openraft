//! Openraft cluster benchmark binary.
//!
//! Run with: cargo run --release --bin bench -- --help

use std::collections::BTreeSet;
use std::fmt::Display;
use std::fmt::Formatter;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use bench_minimal::network::BenchRaft;
use bench_minimal::network::Router;
use bench_minimal::store::ClientRequest;
use clap::Parser;
use openraft::Config;
use tokio::runtime::Builder;
use tokio::runtime::Runtime;
#[cfg(feature = "flamegraph")]
use tracing_flame::FlushGuard;

/// Openraft cluster benchmark
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Number of worker threads for the client runtime
    #[arg(long, default_value_t = 1)]
    client_workers: usize,

    /// Number of worker threads for the server runtime
    #[arg(long, default_value_t = 16)]
    server_workers: usize,

    /// Number of client tasks to spawn
    #[arg(short = 'c', long, default_value_t = 4096, value_parser = parse_underscore_u64)]
    clients: u64,

    /// Total number of operations across all clients
    #[arg(short = 'n', long, default_value_t = 20_000_000, value_parser = parse_underscore_u64)]
    operations: u64,

    /// Number of raft cluster members (1, 3, or 5)
    #[arg(short = 'm', long, default_value_t = 3)]
    members: u64,
}

struct BenchConfig {
    pub client_workers: usize,
    pub server_workers: usize,
    pub n_operations: u64,
    pub n_client: u64,
    pub members: BTreeSet<u64>,
}

impl Display for BenchConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "client_workers: {}, server_workers: {}, clients: {}, n: {}, raft_members: {:?}",
            self.client_workers, self.server_workers, self.n_client, self.n_operations, self.members
        )
    }
}

/// Parse u64 with optional underscores and decimal unit suffix.
///
/// Uses decimal (1000-based) units:
/// - Underscores: "1_000_000"
/// - Suffix k/K: "100k" = 100,000 (thousand)
/// - Suffix m/M: "20m" = 20,000,000 (million)
/// - Suffix g/G: "1g" = 1,000,000,000 (billion)
fn parse_underscore_u64(s: &str) -> Result<u64, String> {
    let s = s.replace('_', "");
    let (num_str, multiplier) = match s.chars().last() {
        Some('k' | 'K') => (&s[..s.len() - 1], 1_000u64),
        Some('m' | 'M') => (&s[..s.len() - 1], 1_000_000u64),
        Some('g' | 'G') => (&s[..s.len() - 1], 1_000_000_000u64),
        _ => (s.as_str(), 1u64),
    };
    let base: u64 = num_str.parse().map_err(|e| format!("{}", e))?;
    Ok(base * multiplier)
}

#[cfg(feature = "flamegraph")]
fn init_flamegraph(path: &str) -> Result<FlushGuard<std::io::BufWriter<std::fs::File>>, tracing_flame::Error> {
    use tracing_flame::FlameLayer;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let (flame_layer, guard) = FlameLayer::with_file(path)?;
    tracing_subscriber::registry().with(flame_layer).init();
    eprintln!("flamegraph profiling enabled, output: {}", path);
    Ok(guard)
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let members: BTreeSet<u64> = (0..args.members).collect();

    let bench_config = BenchConfig {
        client_workers: args.client_workers,
        server_workers: args.server_workers,
        n_operations: args.operations,
        n_client: args.clients,
        members,
    };

    eprintln!("Benchmark config: {}", bench_config);

    bench_with_config(&bench_config)
}

fn bench_with_config(bench_config: &BenchConfig) -> anyhow::Result<()> {
    #[cfg(feature = "tokio-console")]
    {
        console_subscriber::ConsoleLayer::builder()
            .server_addr(([127, 0, 0, 1], 6669))
            .with_default_env()
            .init();
        eprintln!("tokio-console server started on 127.0.0.1:6669");
    }

    #[cfg(feature = "flamegraph")]
    let _flame_guard = init_flamegraph("./flamegraph.folded")?;

    // Server runtime - runs all Raft nodes
    let server_rt = Builder::new_multi_thread()
        .worker_threads(bench_config.server_workers)
        .enable_all()
        .thread_name("bench-server")
        .thread_stack_size(3 * 1024 * 1024)
        .build()?;

    // Client runtime - runs client tasks
    let client_rt = Builder::new_multi_thread()
        .worker_threads(bench_config.client_workers)
        .enable_all()
        .thread_name("bench-client")
        .thread_stack_size(3 * 1024 * 1024)
        .build()?;

    // Create cluster on server runtime
    let (_router, leader) = create_cluster(&server_rt, bench_config)?;

    // Run benchmark on client runtime
    client_rt.block_on(do_bench(bench_config, leader))
}

fn create_cluster(server_rt: &Runtime, bench_config: &BenchConfig) -> anyhow::Result<(Router, BenchRaft)> {
    server_rt.block_on(async {
        let config = Arc::new(
            Config {
                election_timeout_min: 200,
                election_timeout_max: 2000,
                purge_batch_size: 1024,
                max_payload_entries: 1024,
                ..Default::default()
            }
            .validate()?,
        );

        let mut router = Router::new();
        router.new_cluster(config, bench_config.members.clone()).await?;
        let leader = router.get_raft(0);
        Ok((router, leader))
    })
}

/// Benchmark client_write.
///
/// Cluster config:
/// - Log: in-memory BTree
/// - StateMachine: in-memory BTree
async fn do_bench(bench_config: &BenchConfig, leader: BenchRaft) -> anyhow::Result<()> {
    let n_client = bench_config.n_client;
    let ops_per_client = bench_config.n_operations / n_client;
    let total = ops_per_client * n_client;

    let mut handles = Vec::new();

    // Benchmark start
    let now = Instant::now();

    // Spawn stats printing task
    let stats_leader = leader.clone();
    let stats_handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if let Ok(stats) = stats_leader.runtime_stats().await {
                eprintln!("[{:>6.2}s] {}", now.elapsed().as_secs_f64(), stats.display());
            }
        }
    });

    for _client_id in 0..n_client {
        let l = leader.clone();
        let h = tokio::spawn(async move {
            for _i in 0..ops_per_client {
                l.client_write(ClientRequest {})
                    .await
                    .map_err(|e| {
                        eprintln!("client_write error: {:?}", e);
                        e
                    })
                    .unwrap();
            }
        });

        handles.push(h)
    }

    leader.wait(timeout()).applied_index_at_least(Some(total), "commit all written logs").await?;

    let elapsed = now.elapsed();

    for h in handles {
        h.await?;
    }

    // Stop stats printing task
    stats_handle.abort();

    // Print final stats
    let stats = leader.runtime_stats().await?;
    eprintln!("[{:>6.2}s] Final: {}", elapsed.as_secs_f64(), stats.display());

    let millis = elapsed.as_millis().max(1);
    println!(
        "{}: time: {:?}, ns/op: {}, op/ms: {}",
        bench_config,
        elapsed,
        elapsed.as_nanos() / (total as u128),
        (total as u128) / millis,
    );

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(400_000))
}
