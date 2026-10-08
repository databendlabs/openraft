//! Openraft cluster benchmark binary.
//!
//! Run with: cargo run --release --bin bench -- --help

use std::collections::BTreeSet;
use std::fmt::Display;
use std::fmt::Formatter;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::ensure;
use bench_minimal::network::BenchRaft;
use bench_minimal::network::Router;
use bench_minimal::store::ClientRequest;
use clap::Parser;
use futures::TryStreamExt;
use openraft::Config;
use tokio::runtime::Builder;
use tokio::runtime::Runtime;
use tokio::sync::Barrier;
#[cfg(feature = "flamegraph")]
use tracing_flame::FlushGuard;

const WARMUP_WRITES: usize = 1024;
const WAIT_TIMEOUT: Duration = Duration::from_secs(400);

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

    /// Batch size for writes (1 = single writes, >1 = batch writes)
    #[arg(short = 'b', long, default_value_t = 1, value_parser = parse_underscore_u64)]
    batch: u64,

    /// Measure request latency and print JSON results (one sample per write or batch).
    #[arg(long)]
    latency: bool,

    /// JSON object overriding Raft config fields; unknown fields are rejected.
    #[arg(long, default_value = "{}")]
    raft_config: String,

    /// Target operations per second (0 = unpaced); latency includes any arrival backlog.
    #[arg(long, default_value_t = 0, value_parser = parse_underscore_u64)]
    write_rate: u64,

    /// Delay before each log append returns, in microseconds.
    #[arg(long, default_value_t = 0)]
    append_delay_us: u64,

    /// Delay from append return to the asynchronous flush callback, in microseconds.
    #[arg(long, default_value_t = 0)]
    flush_delay_us: u64,

    /// One-way AppendEntries network delay, in microseconds.
    #[arg(long, default_value_t = 0)]
    network_delay_us: u64,

    /// Successful writes before measurement starts.
    #[arg(long, default_value_t = WARMUP_WRITES)]
    warmup_writes: usize,
}

struct BenchConfig {
    pub client_workers: usize,
    pub server_workers: usize,
    pub n_operations: u64,
    pub n_client: u64,
    pub members: BTreeSet<u64>,
    pub batch_size: u64,
    pub latency: bool,
    pub raft_config: serde_json::Value,
    pub write_rate: u64,
    pub append_delay_us: u64,
    pub flush_delay_us: u64,
    pub network_delay_us: u64,
    pub warmup_writes: usize,
}

impl Display for BenchConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "client_workers: {}, server_workers: {}, clients: {}, n: {}, batch: {}, raft_members: {:?}",
            self.client_workers,
            self.server_workers,
            self.n_client,
            format_number(self.n_operations),
            self.batch_size,
            self.members
        )
    }
}

/// Format a number in human-readable form: "20m (20_000_000)"
fn format_number(n: u64) -> String {
    let with_underscores = format_with_underscores(n);
    let short = if n >= 1_000_000_000 && n.is_multiple_of(1_000_000_000) {
        format!("{}g", n / 1_000_000_000)
    } else if n >= 1_000_000 && n.is_multiple_of(1_000_000) {
        format!("{}m", n / 1_000_000)
    } else if n >= 1_000 && n.is_multiple_of(1_000) {
        format!("{}k", n / 1_000)
    } else {
        return with_underscores;
    };
    format!("{} ({})", short, with_underscores)
}

/// Format a number with underscores as thousand separators.
fn format_with_underscores(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push('_');
        }
        result.push(c);
    }
    result.chars().rev().collect()
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
    validate(&args)?;
    let raft_config = serde_json::from_str(&args.raft_config)?;

    let members: BTreeSet<u64> = (0..args.members).collect();

    let bench_config = BenchConfig {
        client_workers: args.client_workers,
        server_workers: args.server_workers,
        n_operations: args.operations,
        n_client: args.clients,
        members,
        batch_size: args.batch,
        latency: args.latency,
        raft_config,
        write_rate: args.write_rate,
        append_delay_us: args.append_delay_us,
        flush_delay_us: args.flush_delay_us,
        network_delay_us: args.network_delay_us,
        warmup_writes: args.warmup_writes,
    };

    eprintln!("Benchmark config: {}", bench_config);

    bench_with_config(&bench_config)
}

fn validate(args: &Args) -> anyhow::Result<()> {
    ensure!(args.clients > 0, "clients must be positive");
    ensure!(args.batch > 0, "batch must be positive");
    ensure!(args.client_workers > 0, "client-workers must be positive");
    ensure!(args.server_workers > 0, "server-workers must be positive");
    let valid_members = [1, 3, 5].contains(&args.members);
    ensure!(valid_members, "members must be 1, 3, or 5");
    let group = args.clients.checked_mul(args.batch);
    let group = group.context("clients * batch overflows")?;
    ensure!(args.operations >= group, "operations must cover every client");
    if args.write_rate > 0 {
        let seconds = args.operations as f64 / args.write_rate as f64;
        let within_timeout = seconds < WAIT_TIMEOUT.as_secs_f64();
        ensure!(within_timeout, "arrival schedule exceeds the benchmark timeout");
    }
    let delays = [args.append_delay_us, args.flush_delay_us, args.network_delay_us];
    let max_delay = delays.into_iter().max().unwrap();
    let within_timeout = u128::from(max_delay) < WAIT_TIMEOUT.as_micros();
    ensure!(within_timeout, "simulated delay exceeds the benchmark timeout");
    Ok(())
}

fn configure(config: Config, overrides: &serde_json::Value) -> anyhow::Result<Config> {
    let overrides = overrides.as_object().context("raft-config must be a JSON object")?;
    let mut config = serde_json::to_value(config)?;
    for (name, value) in overrides {
        let field = config.get_mut(name);
        let field = field.with_context(|| format!("unknown Raft config field: {name}"))?;
        *field = value.clone();
    }
    let config: Config = serde_json::from_value(config)?;
    let config = config.validate()?;
    Ok(config)
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
    client_rt.block_on(async {
        let benchmark = do_bench(bench_config, leader);
        let timed = tokio::time::timeout(WAIT_TIMEOUT, benchmark);
        timed.await?
    })
}

fn create_cluster(server_rt: &Runtime, bench_config: &BenchConfig) -> anyhow::Result<(Router, BenchRaft)> {
    server_rt.block_on(async {
        let config = Config {
            election_timeout_min: 200,
            election_timeout_max: 2000,
            purge_batch_size: 1024,
            max_payload_entries: 1024,
            ..Default::default()
        };
        let config = configure(config, &bench_config.raft_config)?;
        let config = Arc::new(config);

        let append_delay = Duration::from_micros(bench_config.append_delay_us);
        let flush_delay = Duration::from_micros(bench_config.flush_delay_us);
        let network_delay = Duration::from_micros(bench_config.network_delay_us);
        let mut router = Router::with_delays(append_delay, flush_delay, network_delay);
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
    let batch_size = bench_config.batch_size;
    let ops_per_client = bench_config.n_operations / n_client / batch_size * batch_size;
    let total = ops_per_client * n_client;
    let measure_latency = bench_config.latency;
    let client_count = usize::try_from(n_client)?;
    let start = Arc::new(Barrier::new(client_count + 1));
    let epoch = Arc::new(OnceLock::new());
    let write_rate = bench_config.write_rate;

    for _ in 0..bench_config.warmup_writes {
        leader.client_write(ClientRequest {}).await?;
    }

    let mut handles = Vec::new();

    // Spawn stats printing task
    #[cfg(feature = "runtime-stats")]
    let stats_leader = leader.clone();
    #[cfg(feature = "runtime-stats")]
    let stats_handle = tokio::spawn(async move {
        let now = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if let Ok(stats) = stats_leader.runtime_stats().await {
                eprintln!(
                    "[{:>6.2}s] {}",
                    now.elapsed().as_secs_f64(),
                    stats.display().multiline()
                );
            }
        }
    });

    for client_id in 0..n_client {
        let l = leader.clone();
        let start = start.clone();
        let epoch = epoch.clone();
        let h = if batch_size <= 1 {
            // Single write mode
            tokio::spawn(async move {
                let mut latencies = Vec::new();
                start.wait().await;
                let epoch = *epoch.get().unwrap();
                for i in 0..ops_per_client {
                    let operation = i * n_client + client_id;
                    let started = if write_rate > 0 {
                        let scheduled = pace(epoch, operation, write_rate).await;
                        measure_latency.then_some(scheduled)
                    } else {
                        measure_latency.then(Instant::now)
                    };
                    l.client_write(ClientRequest {}).await?;
                    if let Some(started) = started {
                        let elapsed = started.elapsed();
                        let nanos = elapsed.as_nanos();
                        let nanos = u64::try_from(nanos)?;
                        latencies.push(nanos);
                    }
                }
                Ok::<_, anyhow::Error>(latencies)
            })
        } else {
            // Batch write mode
            tokio::spawn(async move {
                let batches = ops_per_client / batch_size;
                let mut latencies = Vec::new();
                start.wait().await;
                let epoch = *epoch.get().unwrap();

                for batch in 0..batches {
                    let requests: Vec<_> = (0..batch_size).map(|_| ClientRequest {}).collect();
                    let operation = (batch * n_client + client_id) * batch_size;
                    let started = if write_rate > 0 {
                        let scheduled = pace(epoch, operation, write_rate).await;
                        measure_latency.then_some(scheduled)
                    } else {
                        measure_latency.then(Instant::now)
                    };
                    let mut stream = l.client_write_many(requests).await?;
                    while let Some(result) = stream.try_next().await? {
                        result?;
                    }
                    if let Some(started) = started {
                        let elapsed = started.elapsed();
                        let nanos = elapsed.as_nanos();
                        let nanos = u64::try_from(nanos)?;
                        latencies.push(nanos);
                    }
                }
                Ok::<_, anyhow::Error>(latencies)
            })
        };

        handles.push(h)
    }

    let now = Instant::now();
    epoch.set(now).unwrap();
    start.wait().await;
    let mut client_latencies = Vec::with_capacity(client_count);
    for h in handles {
        let result = h.await?;
        let samples = result?;
        client_latencies.push(samples);
    }
    let elapsed = now.elapsed();

    // Stop stats printing task
    #[cfg(feature = "runtime-stats")]
    stats_handle.abort();

    // Print final stats
    #[cfg(feature = "runtime-stats")]
    {
        let stats = leader.runtime_stats().await?;
        eprintln!(
            "[{:>6.2}s] Final:\n{}",
            elapsed.as_secs_f64(),
            stats.display().human_readable()
        );
    }

    if measure_latency {
        let mut latencies = Vec::new();
        for samples in client_latencies {
            latencies.extend(samples);
        }
        latencies.sort_unstable();
        let seconds = elapsed.as_secs_f64();
        let operations_per_second = total as f64 / seconds;
        let p50 = percentile(&latencies, 50);
        let p95 = percentile(&latencies, 95);
        let p99 = percentile(&latencies, 99);
        let samples = latencies.len();
        let total_latency: u128 = latencies.iter().map(|value| u128::from(*value)).sum();
        let mean = total_latency / samples as u128;
        let mean = u64::try_from(mean)?;
        let runtime_stats = cfg!(feature = "runtime-stats");
        let latency_origin = if write_rate > 0 {
            "scheduled_arrival"
        } else {
            "request_start"
        };
        let report = serde_json::json!({
            "requested_operations": bench_config.n_operations,
            "operations": total,
            "samples": samples,
            "seconds": seconds,
            "operations_per_second": operations_per_second,
            "request_p50_ns": p50,
            "request_mean_ns": mean,
            "request_p95_ns": p95,
            "request_p99_ns": p99,
            "clients": n_client,
            "batch": batch_size,
            "members": bench_config.members,
            "client_workers": bench_config.client_workers,
            "server_workers": bench_config.server_workers,
            "runtime_stats": runtime_stats,
            "latency_origin": latency_origin,
            "warmup_writes": bench_config.warmup_writes,
            "raft_config": bench_config.raft_config,
            "write_rate": write_rate,
            "append_delay_us": bench_config.append_delay_us,
            "flush_delay_us": bench_config.flush_delay_us,
            "network_delay_us": bench_config.network_delay_us,
        });
        println!("{report}");
        return Ok(());
    }

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

async fn pace(epoch: Instant, operation: u64, write_rate: u64) -> Instant {
    let seconds = operation as f64 / write_rate as f64;
    let offset = Duration::from_secs_f64(seconds);
    let scheduled = epoch + offset;
    let deadline = tokio::time::Instant::from_std(scheduled);
    tokio::time::sleep_until(deadline).await;
    scheduled
}

fn percentile(samples: &[u64], percentile: usize) -> u64 {
    let nonempty = !samples.is_empty();
    assert!(nonempty, "latency samples must not be empty");
    let valid = (1..=100).contains(&percentile);
    assert!(valid, "percentile must be between 1 and 100");
    let rank = samples.len() * percentile;
    let rank = rank.div_ceil(100);
    samples[rank - 1]
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::pace;
    use super::percentile;

    #[tokio::test]
    async fn latency_includes_arrival_backlog() {
        let epoch = Instant::now() - Duration::from_secs(1);
        let started = pace(epoch, 25, 1000).await;
        let expected = epoch + Duration::from_millis(25);
        assert_eq!(expected, started);
    }

    #[test]
    fn nearest_rank_percentiles() {
        let samples: Vec<u64> = (1..=100).collect();
        let p50 = percentile(&samples, 50);
        assert_eq!(50, p50);
        let p95 = percentile(&samples, 95);
        assert_eq!(95, p95);
        let p99 = percentile(&samples, 99);
        assert_eq!(99, p99);
        let p99 = percentile(&[100, 200, 300], 99);
        assert_eq!(300, p99);
    }
}
