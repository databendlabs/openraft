//! Openraft cluster benchmark binary.
//!
//! Run with: cargo run --release --bin bench -- --help

use std::collections::BTreeSet;
use std::fmt::Display;
use std::fmt::Formatter;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use clap::Parser;
use cluster_benchmark::network::Router;
use cluster_benchmark::store::ClientRequest;
use openraft::Config;
use tokio::runtime::Builder;

#[cfg(feature = "flamegraph")]
use tracing_flame::FlushGuard;

/// Openraft cluster benchmark
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Number of worker threads for the tokio runtime
    #[arg(short = 'w', long, default_value_t = 32)]
    workers: usize,

    /// Number of client tasks to spawn
    #[arg(short = 'c', long, default_value_t = 256)]
    clients: u64,

    /// Number of operations per client
    #[arg(short = 'n', long, default_value_t = 100_000)]
    operations: u64,

    /// Number of raft cluster members (1, 3, or 5)
    #[arg(short = 'm', long, default_value_t = 3)]
    members: u64,
}

struct BenchConfig {
    pub worker_threads: usize,
    pub n_operations: u64,
    pub n_client: u64,
    pub members: BTreeSet<u64>,
}

impl Display for BenchConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "workers: {}, clients: {}, n: {}, raft_members: {:?}",
            self.worker_threads, self.n_client, self.n_operations, self.members
        )
    }
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
        worker_threads: args.workers,
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

    let rt = Builder::new_multi_thread()
        .worker_threads(bench_config.worker_threads)
        .enable_all()
        .thread_name("bench-cluster")
        .thread_stack_size(3 * 1024 * 1024)
        .build()?;

    rt.block_on(do_bench(bench_config))
}

/// Benchmark client_write.
///
/// Cluster config:
/// - Log: in-memory BTree
/// - StateMachine: in-memory BTree
async fn do_bench(bench_config: &BenchConfig) -> anyhow::Result<()> {
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
    router.new_cluster(config.clone(), bench_config.members.clone()).await?;

    let n = bench_config.n_operations;
    let total = n * bench_config.n_client;

    let leader = router.get_raft(0);
    let mut handles = Vec::new();

    // Benchmark start
    let now = Instant::now();

    // Spawn stats printing task
    let stats_leader = leader.clone();
    let stats_handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let stats = stats_leader.runtime_stats();
            let display = stats.with_mut(|s| s.display());
            eprintln!("[{:>6.2}s] {}", now.elapsed().as_secs_f64(), display);
        }
    });

    for _nc in 0..bench_config.n_client {
        let l = leader.clone();
        let h = tokio::spawn(async move {
            for _i in 0..n {
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
    let stats = leader.runtime_stats();
    let display = stats.with_mut(|s| s.display());
    eprintln!("[{:>6.2}s] Final: {}", elapsed.as_secs_f64(), display);

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
