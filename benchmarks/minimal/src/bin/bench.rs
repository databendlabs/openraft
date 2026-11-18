//! Openraft cluster benchmark binary.
//!
//! Run with: cargo run --release --bin bench -- --help

#[path = "bench/apply_lag.rs"]
mod apply_lag;
#[path = "bench/leader_change.rs"]
mod leader_change;
#[path = "bench/read_workload.rs"]
mod read_workload;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Display;
use std::fmt::Formatter;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::ensure;
use apply_lag::ApplyLag;
use bench_minimal::network::BenchRaft;
use bench_minimal::network::Router;
use bench_minimal::store::ClientRequest;
use clap::Parser;
use leader_change::Leaders;
use openraft::Config;
use openraft::ReadPolicy;
use read_workload::ReadWorkload;
use tokio::runtime::Builder;
use tokio::runtime::Runtime;
use tokio::sync::Barrier;
#[cfg(feature = "flamegraph")]
use tracing_flame::FlushGuard;

#[path = "bench/arrival.rs"]
mod arrival;
#[path = "bench/request_sample.rs"]
mod request_sample;
#[path = "bench/write_workload.rs"]
mod write_workload;

use request_sample::Samples;
use write_workload::WriteWorkload;

#[path = "bench/apply_metrics.rs"]
mod apply_metrics;
#[path = "bench/histogram.rs"]
mod histogram;
#[path = "bench/measurement.rs"]
mod measurement;
#[path = "bench/run_monitor.rs"]
mod run_monitor;
#[path = "bench/summary.rs"]
mod summary;
#[path = "bench/write_progress.rs"]
mod write_progress;

#[path = "bench/profile_history.rs"]
mod profile_history;

#[cfg(feature = "apply-pipeline")]
#[path = "bench/apply_pipeline.rs"]
mod apply_pipeline;

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

    /// Number of write client tasks to spawn
    #[arg(short = 'c', long, default_value_t = 4096, value_parser = parse_underscore_u64)]
    clients: u64,

    /// Total number of write operations across all write clients
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

    /// Number of concurrent lease-read clients (0 keeps the write-only benchmark).
    #[arg(long, default_value_t = 0, value_parser = parse_underscore_u64)]
    read_clients: u64,

    /// Total lease-read attempts across all read clients.
    #[arg(long, default_value_t = 0, value_parser = parse_underscore_u64)]
    read_operations: u64,

    /// Target read attempts per second (0 = unpaced).
    #[arg(long, default_value_t = 0, value_parser = parse_underscore_u64)]
    read_rate: u64,

    /// Wait for the linearizer's apply barrier after checking the lease.
    #[arg(long)]
    read_await_ready: bool,

    /// Query a coherent state-machine write counter after waiting for the read barrier.
    #[arg(long)]
    read_query: bool,

    /// Request a relaxed linearizer before the query (requires --read-query).
    #[arg(long)]
    read_relaxed: bool,

    /// How the leader confirms leadership for each read [default: lease].
    #[arg(long, value_enum)]
    read_policy: Option<ReadPolicyArg>,

    /// Await each read's barrier and run its query on this follower instead of the leader
    /// (requires --read-query); also reports how far the follower's apply trails the leader's.
    #[arg(long)]
    read_on_follower: Option<u64>,

    /// Report read failures separately instead of stopping at the first one.
    #[arg(long)]
    allow_read_errors: bool,

    /// Idle time after warmup, before clients start; useful for expired-lease controls.
    #[arg(long, default_value_t = 0)]
    read_delay_ms: u64,

    /// Transfer leadership from node 0 to node 1 this long after clients start; clients follow
    /// the new leader, and the report measures reads around the election.
    #[arg(long)]
    transfer_leader_after_ms: Option<u64>,

    /// Delay before each log append returns, in microseconds.
    #[arg(long, default_value_t = 0)]
    append_delay_us: u64,

    /// Delay from append return to the asynchronous flush callback, in microseconds.
    #[arg(long, default_value_t = 0)]
    flush_delay_us: u64,

    /// One-way AppendEntries network delay, in microseconds.
    #[arg(long, default_value_t = 0)]
    network_delay_us: u64,

    /// Delay of every log read the leader issues for replication, in microseconds.
    #[arg(long, default_value_t = 0)]
    replication_read_delay_us: u64,

    /// Delay of the log read that loads each apply batch, in microseconds.
    #[arg(long, default_value_t = 0)]
    apply_read_delay_us: u64,

    /// State-machine I/O delay of each apply batch, in microseconds; responses wait for it.
    #[arg(long, default_value_t = 0)]
    apply_io_delay_us: u64,

    /// State-machine delay of each applied entry, in microseconds; an `apply()` call of n
    /// entries waits n times this, and responses wait for it.
    #[arg(long, default_value_t = 0)]
    apply_entry_delay_us: u64,

    /// Successful writes before measurement starts.
    #[arg(long, default_value_t = WARMUP_WRITES)]
    warmup_writes: usize,

    #[command(flatten)]
    arrival: arrival::ArrivalConfig,

    #[command(flatten)]
    measurement: measurement::MeasurementConfig,
    #[command(flatten)]
    profile: bench_minimal::profile::Profile,
}

/// The read policy of each benchmark read.
#[derive(Clone, Copy, Debug, Default, clap::ValueEnum, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
enum ReadPolicyArg {
    /// `ReadPolicy::LeaseRead`; cached outside RaftCore when `enable_local_lease_read` is set.
    #[default]
    Lease,
    /// `ReadPolicy::ReadIndex`; every read waits for a quorum to confirm leadership.
    ReadIndex,
}

impl From<ReadPolicyArg> for ReadPolicy {
    fn from(policy: ReadPolicyArg) -> Self {
        match policy {
            ReadPolicyArg::Lease => ReadPolicy::LeaseRead,
            ReadPolicyArg::ReadIndex => ReadPolicy::ReadIndex,
        }
    }
}

#[derive(Clone, serde::Serialize)]
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
    pub read_clients: u64,
    pub read_operations: u64,
    pub read_rate: u64,
    pub read_await_ready: bool,
    pub read_query: bool,
    pub read_relaxed: bool,
    pub read_policy: ReadPolicyArg,
    pub read_on_follower: Option<u64>,
    pub allow_read_errors: bool,
    pub read_delay_ms: u64,
    pub transfer_leader_after_ms: Option<u64>,
    pub append_delay_us: u64,
    pub flush_delay_us: u64,
    pub network_delay_us: u64,
    pub replication_read_delay_us: u64,
    pub apply_read_delay_us: u64,
    pub apply_io_delay_us: u64,
    pub apply_entry_delay_us: u64,
    pub warmup_writes: usize,
    pub arrival: arrival::ArrivalConfig,
    pub measurement: measurement::MeasurementConfig,
    pub profile: bench_minimal::profile::Profile,
}

impl BenchConfig {
    fn operations_per_client(&self, id: u64) -> u64 {
        if self.n_client == 0 {
            return 0;
        }
        let batches = self.n_operations / self.batch_size;
        client_operations(batches, self.n_client, id) * self.batch_size
    }

    fn trace_budget(&self) -> usize {
        let clients = (self.n_client + self.read_clients) as usize;
        self.measurement.trace_budget(clients)
    }
}

impl Display for BenchConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let compiled = compiled_features();
        write!(
            f,
            "client_workers: {}, server_workers: {}, clients: {}, n: {}, batch: {}, raft_members: {:?}, compiled_features: {}",
            self.client_workers,
            self.server_workers,
            self.n_client,
            format_number(self.n_operations),
            self.batch_size,
            self.members,
            compiled
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
        read_clients: args.read_clients,
        read_operations: args.read_operations,
        read_rate: args.read_rate,
        read_await_ready: args.read_await_ready,
        read_query: args.read_query,
        read_relaxed: args.read_relaxed,
        read_policy: args.read_policy.unwrap_or_default(),
        read_on_follower: args.read_on_follower,
        allow_read_errors: args.allow_read_errors,
        read_delay_ms: args.read_delay_ms,
        transfer_leader_after_ms: args.transfer_leader_after_ms,
        append_delay_us: args.append_delay_us,
        flush_delay_us: args.flush_delay_us,
        network_delay_us: args.network_delay_us,
        replication_read_delay_us: args.replication_read_delay_us,
        apply_read_delay_us: args.apply_read_delay_us,
        apply_io_delay_us: args.apply_io_delay_us,
        apply_entry_delay_us: args.apply_entry_delay_us,
        warmup_writes: args.warmup_writes,
        arrival: args.arrival.clone(),
        profile: args.profile.clone(),
        measurement: args.measurement.clone(),
    };

    eprintln!("Benchmark config: {}", bench_config);

    bench_with_config(&bench_config)
}

fn validate(args: &Args) -> anyhow::Result<()> {
    let has_clients = args.clients > 0 || args.read_clients > 0;
    ensure!(has_clients, "at least one write or read client is required");
    if args.clients == 0 {
        ensure!(args.operations == 0, "operations must be zero without write clients");
        ensure!(args.write_rate == 0, "write-rate must be zero without write clients");
    }
    args.measurement.validate(WAIT_TIMEOUT)?;
    args.profile.validate()?;
    if args.profile.profile && args.read_clients > 0 {
        ensure!(args.read_query, "profile readers require --read-query");
    }
    validate_arrivals(args)?;
    validate_reads(args)?;
    validate_leader_change(args)?;
    ensure!(args.batch > 0, "batch must be positive");
    ensure!(args.client_workers > 0, "client-workers must be positive");
    ensure!(args.server_workers > 0, "server-workers must be positive");
    let valid_members = [1, 3, 5].contains(&args.members);
    ensure!(valid_members, "members must be 1, 3, or 5");
    let group = args.clients.checked_mul(args.batch);
    let group = group.context("clients * batch overflows")?;
    if args.measurement.measurement_seconds.is_none() {
        ensure!(args.operations >= group, "operations must cover every client");
        ensure!(
            args.operations.is_multiple_of(args.batch),
            "operations must be divisible by batch"
        );
    }
    validate_schedule(args)?;
    let delays = [
        args.append_delay_us,
        args.flush_delay_us,
        args.network_delay_us,
        args.replication_read_delay_us,
        args.apply_read_delay_us,
        args.apply_io_delay_us,
        args.apply_entry_delay_us,
    ];
    let max_delay = delays.into_iter().max().unwrap();
    let within_timeout = u128::from(max_delay) < WAIT_TIMEOUT.as_micros();
    ensure!(within_timeout, "simulated delay exceeds the benchmark timeout");
    Ok(())
}

fn validate_arrivals(args: &Args) -> anyhow::Result<()> {
    let config = &args.arrival;
    ensure!(config.queue_capacity > 0, "queue-capacity must be positive");
    ensure!(config.burst_size > 0, "burst-size must be positive");
    let timeout = Duration::from_millis(config.request_timeout_ms);
    ensure!(timeout < WAIT_TIMEOUT, "request-timeout-ms exceeds benchmark timeout");
    let measured = args.latency || args.read_clients > 0 || args.measurement.measurement_seconds.is_some();
    let needs_measurement =
        config.open_loop || config.arrival_noop || config.phase_sample_every > 0 || config.request_timeout_ms > 0;
    ensure!(
        !needs_measurement || measured,
        "arrival controls require --latency, read clients, or --measurement-seconds"
    );
    if config.open_loop {
        ensure!(
            args.clients == 0 || args.write_rate > 0,
            "open-loop writes require write-rate"
        );
        ensure!(
            args.read_clients == 0 || args.read_rate > 0,
            "open-loop reads require read-rate"
        );
    }
    Ok(())
}

fn validate_reads(args: &Args) -> anyhow::Result<()> {
    ensure!(
        !args.read_relaxed || args.read_query,
        "read-relaxed requires read-query"
    );
    if let Some(follower) = args.read_on_follower {
        ensure!(args.read_query, "read-on-follower requires read-query");
        let is_follower = (1..args.members).contains(&follower);
        ensure!(is_follower, "read-on-follower must name a follower, 1..members");
    }
    if args.read_clients == 0 {
        ensure!(args.read_operations == 0, "read-operations requires read-clients");
        ensure!(args.read_rate == 0, "read-rate requires read-clients");
        ensure!(args.read_delay_ms == 0, "read-delay-ms requires read-clients");
        ensure!(!args.read_await_ready, "read-await-ready requires read-clients");
        ensure!(!args.read_query, "read-query requires read-clients");
        ensure!(!args.read_relaxed, "read-relaxed requires read-clients");
        ensure!(args.read_policy.is_none(), "read-policy requires read-clients");
        ensure!(!args.allow_read_errors, "allow-read-errors requires read-clients");
        return Ok(());
    }
    if args.measurement.measurement_seconds.is_none() {
        ensure!(
            args.read_operations >= args.read_clients,
            "read-operations must cover every read client"
        );
    }
    Ok(())
}

fn validate_leader_change(args: &Args) -> anyhow::Result<()> {
    if args.transfer_leader_after_ms.is_none() {
        return Ok(());
    }
    ensure!(args.read_clients > 0, "transfer-leader-after-ms requires read-clients");
    ensure!(
        args.members > 1,
        "transfer-leader-after-ms requires more than one member"
    );
    ensure!(args.batch == 1, "transfer-leader-after-ms requires batch 1");
    ensure!(
        args.read_on_follower.is_none(),
        "transfer-leader-after-ms cannot read on a follower"
    );
    Ok(())
}

fn validate_schedule(args: &Args) -> anyhow::Result<()> {
    if args.measurement.measurement_seconds.is_some() {
        let bounds = args.measurement.bounds();
        let idle = Duration::from_millis(args.read_delay_ms);
        let total = idle + bounds.deadline.unwrap();
        ensure!(
            total < WAIT_TIMEOUT,
            "idle time plus measurement window and drain exceed benchmark timeout"
        );
        if let Some(after) = args.transfer_leader_after_ms {
            ensure!(
                Duration::from_millis(after) < bounds.end.unwrap(),
                "leadership transfer must start before measurement ends"
            );
        }
        return Ok(());
    }
    let write_seconds = if args.write_rate > 0 {
        args.operations as f64 / args.write_rate as f64
    } else {
        0.0
    };
    let read_seconds = if args.read_rate > 0 {
        args.read_operations as f64 / args.read_rate as f64
    } else {
        0.0
    };
    let transfer_seconds = args.transfer_leader_after_ms.map_or(0.0, |ms| ms as f64 / 1000.0);
    let workload_seconds = write_seconds.max(read_seconds).max(transfer_seconds);
    let idle = Duration::from_millis(args.read_delay_ms);
    let seconds = idle.as_secs_f64() + workload_seconds;
    let within_timeout = seconds < WAIT_TIMEOUT.as_secs_f64();
    ensure!(
        within_timeout,
        "arrival schedule plus read delay exceeds the benchmark timeout"
    );
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

    let result = (|| -> anyhow::Result<()> {
        let (router, leader) = create_cluster(&server_rt, bench_config)?;
        let server_handle = server_rt.handle().clone();
        client_rt.block_on(async {
            let benchmark = do_bench(bench_config, leader, &router, server_handle);
            let timed = tokio::time::timeout(WAIT_TIMEOUT, benchmark);
            timed.await?
        })
    })();
    if bench_config.measurement.measurement_seconds.is_some() {
        client_rt.shutdown_timeout(Duration::ZERO);
        server_rt.shutdown_timeout(Duration::ZERO);
    }
    result
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
        let router = Router::with_delays(append_delay, flush_delay, network_delay);
        let replication_read_delay = Duration::from_micros(bench_config.replication_read_delay_us);
        let router = router.with_replication_read_delay(replication_read_delay);
        let apply_read_delay = Duration::from_micros(bench_config.apply_read_delay_us);
        let apply_io_delay = Duration::from_micros(bench_config.apply_io_delay_us);
        let apply_entry_delay = Duration::from_micros(bench_config.apply_entry_delay_us);
        let router = router.with_apply_delays(apply_read_delay, apply_io_delay, apply_entry_delay);
        let mut router = router.with_profile(bench_config.profile.clone());
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
async fn do_bench(
    bench_config: &BenchConfig,
    leader: BenchRaft,
    router: &Router,
    server: tokio::runtime::Handle,
) -> anyhow::Result<()> {
    let n_client = bench_config.n_client;
    let batch_size = bench_config.batch_size;
    let bounds = bench_config.measurement.bounds();
    let measure_latency = bench_config.latency
        || bench_config.read_clients > 0
        || bounds.end.is_some()
        || bench_config.profile.profile
        || bench_config.measurement.pipeline_enabled();
    let client_count = usize::try_from(n_client)?;
    let read_clients = usize::try_from(bench_config.read_clients)?;
    let participants = client_count.checked_add(read_clients);
    let participants = participants.context("total clients overflows")?;
    let generators = if bench_config.arrival.open_loop {
        usize::from(n_client > 0) + usize::from(read_clients > 0)
    } else {
        0
    };
    let participants = participants.checked_add(generators).context("generator participants overflows")?;
    let participants = participants.checked_add(1).context("barrier participants overflows")?;
    let start = Arc::new(Barrier::new(participants));
    let epoch = Arc::new(OnceLock::new());
    let write_rate = bench_config.write_rate;
    let write_live = Arc::new(run_monitor::Live::default());
    let read_live = Arc::new(run_monitor::Live::default());
    let history = if bench_config.profile.profile {
        Some(Arc::new(profile_history::History::new(bench_config)))
    } else {
        None
    };

    #[cfg(feature = "apply-pipeline")]
    let pipeline = apply_pipeline::install(router, bench_config, epoch.clone()).await?;
    for _ in 0..bench_config.warmup_writes {
        leader.client_write(ClientRequest::default()).await?;
    }

    if bench_config.read_delay_ms > 0 {
        let delay = Duration::from_millis(bench_config.read_delay_ms);
        tokio::time::sleep(delay).await;
    }
    let mut diagnostics = BTreeMap::new();
    let mut metric_watchers = Vec::new();
    if bench_config.measurement.diagnostics() {
        for id in &bench_config.members {
            let raft = router.get_raft(*id);
            let metrics = Arc::new(apply_metrics::ApplyMetrics::new(epoch.clone(), bounds));
            let source = metrics.clone();
            let machine = raft.with_state_machine(|sm| Box::pin(async move { sm.clone() })).await?;
            machine
                .observe_applies(Arc::new(move |index, entries, at| {
                    source.source_applied(index, entries, at)
                }))
                .map_err(anyhow::Error::msg)?;
            raft.set_metrics_recorder(Some(metrics.clone())).await?;
            tokio::time::timeout(WAIT_TIMEOUT, metrics.wait_installed()).await?;
            metric_watchers.push(metrics.watch(&server, raft));
            diagnostics.insert(*id, metrics);
        }
    }

    let leaders = match bench_config.transfer_leader_after_ms {
        Some(_) => {
            let mut machines = BTreeMap::new();
            for id in bench_config.members.iter().copied() {
                let raft = router.get_raft(id);
                let machine = raft.with_state_machine(|sm| Box::pin(async move { sm.clone() })).await?;
                machines.insert(id, machine);
            }
            Some(Leaders::new(router.clone(), machines))
        }
        None => None,
    };
    let read_workload = if read_clients > 0 {
        let reader = match bench_config.read_on_follower {
            Some(follower) => router.get_raft(follower),
            None => leader.clone(),
        };
        let state_machine = reader.with_state_machine(|sm| Box::pin(async move { sm.clone() })).await?;
        Some(ReadWorkload {
            config: Arc::new(bench_config.clone()),
            leader: leader.clone(),
            reader,
            state_machine,
            leaders: leaders.clone(),
            start: start.clone(),
            epoch: epoch.clone(),
            live: read_live.clone(),
            history: history.clone(),
        })
    } else {
        None
    };
    let apply_lag = match &read_workload {
        Some(workload) if bench_config.read_on_follower.is_some() => Some(ApplyLag::start(
            &workload.leader,
            &workload.reader,
            epoch.clone(),
            bounds,
        )),
        _ => None,
    };
    let read_handles = match &read_workload {
        Some(workload) => workload.spawn_clients(),
        None => Vec::new(),
    };
    let write_workload = WriteWorkload {
        config: Arc::new(bench_config.clone()),
        leader: leader.clone(),
        leaders: leaders.clone(),
        start: start.clone(),
        epoch: epoch.clone(),
        live: write_live.clone(),
        history: history.clone(),
    };
    let handles = write_workload.spawn_clients();

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

    #[cfg(feature = "runtime-stats")]
    let command_before = leader.runtime_stats().await?.command_counts;
    let cpu_before = if cfg!(unix) && bounds.end.is_none() {
        Some(run_monitor::usage()?)
    } else {
        None
    };
    let now = Instant::now();
    epoch.set(now).unwrap();
    #[cfg(feature = "runtime-stats")]
    let command_window = if bounds.end.is_some() {
        Some(tokio::spawn(run_monitor::command_window(leader.clone(), now, bounds)))
    } else {
        None
    };
    let monitor = if bounds.end.is_some() {
        Some(run_monitor::RunMonitor::start(now, bounds, write_live, read_live))
    } else {
        None
    };
    let transfer = match (&leaders, bench_config.transfer_leader_after_ms) {
        (Some(leaders), Some(after)) => Some(leaders.transfer_after(now, Duration::from_millis(after))),
        _ => None,
    };
    start.wait().await;
    let writes = collect_clients(handles);
    let reads = collect_clients(read_handles);
    let (write_groups, read_groups) = tokio::try_join!(writes, reads)?;
    let write_workers = write_groups.len();
    let read_workers = read_groups.len();
    if let Some(end) = bounds.end {
        tokio::time::sleep_until(tokio::time::Instant::from_std(now + end)).await;
    }
    let elapsed = now.elapsed();
    let cpu_after = if cpu_before.is_some() {
        Some(run_monitor::usage()?)
    } else {
        None
    };
    let completed_writes: u64 = write_groups.iter().map(|samples| samples.window_completed).sum();
    let completed_reads: u64 = read_groups.iter().map(|samples| samples.window_completed).sum();
    let completed_window = completed_writes + completed_reads;
    let monitoring = match monitor {
        Some(monitor) => monitor.finish(completed_window).await?,
        None => match cpu_before {
            Some(before) => {
                let after = cpu_after.unwrap();
                serde_json::json!({"resources":{"cpu_seconds":after.0-before.0,"cpu_ns_per_completed_operation":if completed_window==0 { serde_json::Value::Null } else { serde_json::json!((after.0-before.0)*1e9/completed_window as f64) },
                    "process_peak_rss_bytes_at_end":after.1,"scope":"whole_process_workload_before_json_reporting","rss_scope":"high_water_mark_since_process_start"}})
            }
            None => serde_json::Value::Null,
        },
    };
    let client_latencies = merge_samples(write_groups, bounds, bench_config.measurement.trace_limit);
    let read_samples = merge_samples(read_groups, bounds, bench_config.measurement.trace_limit);
    let follower_lag = match apply_lag {
        Some(apply_lag) => Some(apply_lag.finish().await?),
        None => None,
    };
    let leader_change = match transfer {
        Some(mut transfer) => match bounds.deadline {
            Some(deadline) => {
                match tokio::time::timeout_at(tokio::time::Instant::from_std(now + deadline), &mut transfer).await {
                    Ok(change) => Some(leaders.as_ref().unwrap().report(&change??)),
                    Err(_) => {
                        transfer.abort();
                        Some(serde_json::json!({"status":"pending_at_drain_deadline"}))
                    }
                }
            }
            None => Some(leaders.as_ref().unwrap().report(&transfer.await??)),
        },
        None => None,
    };

    for (stop, mut watcher) in metric_watchers {
        stop.send_replace(true);
        match bounds.deadline {
            Some(deadline) => {
                match tokio::time::timeout_at(tokio::time::Instant::from_std(now + deadline), &mut watcher).await {
                    Ok(result) => result??,
                    Err(_) => watcher.abort(),
                }
            }
            None => watcher.await??,
        }
    }
    let apply_metrics: BTreeMap<_, _> = diagnostics.iter().map(|(id, metrics)| (*id, metrics.report())).collect();
    #[cfg(feature = "apply-pipeline")]
    let apply_pipeline: BTreeMap<_, _> = pipeline.iter().map(|(id, probe)| (*id, probe.report())).collect();
    #[cfg(not(feature = "apply-pipeline"))]
    let apply_pipeline = serde_json::Value::Null;
    #[cfg(feature = "runtime-stats")]
    let command_counts = match command_window {
        Some(mut task) => match tokio::time::timeout_at(
            tokio::time::Instant::from_std(now + bounds.deadline.unwrap()),
            &mut task,
        )
        .await
        {
            Ok(result) => result??,
            Err(_) => {
                task.abort();
                serde_json::json!({"status":"core_snapshot_pending_at_drain_deadline"})
            }
        },
        None => {
            let after = leader.runtime_stats().await?.command_counts;
            let mut report = run_monitor::command_delta(&command_before, &after);
            report["scope"] = serde_json::json!("after_bootstrap_until_clients_drained");
            report
        }
    };
    #[cfg(not(feature = "runtime-stats"))]
    let command_counts = serde_json::Value::Null;

    // Stop stats printing task
    #[cfg(feature = "runtime-stats")]
    stats_handle.abort();

    // Print final stats
    #[cfg(feature = "runtime-stats")]
    if bounds.end.is_none() {
        let stats = leader.runtime_stats().await?;
        eprintln!(
            "[{:>6.2}s] Final:\n{}",
            elapsed.as_secs_f64(),
            stats.display().human_readable()
        );
    }

    let total = client_latencies.completed();
    let memory =
        serde_json::json!({"write":client_latencies.memory(write_workers),"read":read_samples.memory(read_workers)});
    let window = bounds.report(elapsed);
    let resolved_config = serde_json::to_value(leader.config())?;
    let profile_report = if bench_config.profile.profile {
        let outcomes = client_latencies.timing();
        let pending = outcomes["all_phase_outcomes"]["pending"].as_u64().context("missing pending write count")?;
        let uncertain =
            outcomes["all_phase_outcomes"]["timed_out"].as_u64().context("missing timed-out write count")?;
        let failed = outcomes["all_phase_outcomes"]["failed"].as_u64().context("missing failed write count")?;
        let mut report = router.verify_profile(pending == 0 && uncertain == 0 && failed == 0).await?;
        let history = history.as_ref().expect("profile history");
        report["acknowledgement_verification"] = history.verify_final(router, &bench_config.members)?;
        report["history"] = history.report();
        report
    } else {
        serde_json::Value::Null
    };
    if read_workload.is_some() {
        let writes = client_latencies.write_report(elapsed);
        let reads = read_samples.read_report(elapsed);
        let timing = client_latencies.timing();
        let local = leader.config().enable_local_lease_read.unwrap_or(false);
        let mut report = serde_json::json!({"schema_version":2,"operation_units":{"write":"entry","read":"request"},"seconds":elapsed.as_secs_f64(),"write":writes,"read":reads,
            "write_timing":timing,"config":bench_config,"local_lease_read_enabled":local,"runtime_stats":cfg!(feature="runtime-stats"),"compiled_features":compiled_features(),
            "profile":profile_report,"window":window,"monitoring":monitoring,"memory":memory,"apply_metrics":apply_metrics,
            "command_counts":command_counts,"resolved_raft_config":resolved_config,"follower_lag":follower_lag,"leader_change":leader_change});
        report["apply_pipeline"] = serde_json::to_value(apply_pipeline)?;
        println!("{report}");
        return Ok(());
    }
    if measure_latency {
        let summary = client_latencies.write_report(elapsed);
        let timing = client_latencies.timing();
        let histogram = &client_latencies.successful;
        let request_latency = histogram.latency();
        let requested = if bounds.end.is_none() {
            Some(bench_config.n_operations)
        } else {
            None
        };
        let mut report = serde_json::json!({"schema_version":2,"operation_unit":"entry","latency_unit":"request_or_batch","requested_operations":requested,"operations":total,
            "operations_per_second":summary["operations_per_second"],"operations_per_second_through_drain":summary["operations_per_second_through_drain"],
            "completed_in_window":summary["completed_in_window"],"samples":histogram.count,"seconds":elapsed.as_secs_f64(),
            "request_p50_ns":request_latency["p50_ns"],"request_mean_ns":request_latency["mean_ns"],
            "request_p95_ns":request_latency["p95_ns"],"request_p99_ns":request_latency["p99_ns"],
            "timing":timing,"profile":profile_report,"window":window,"monitoring":monitoring,"memory":memory,"apply_metrics":apply_metrics,"command_counts":command_counts,
            "clients":n_client,"batch":batch_size,"members":bench_config.members,"client_workers":bench_config.client_workers,
            "server_workers":bench_config.server_workers,"runtime_stats":cfg!(feature="runtime-stats"),
            "latency_origin":if write_rate>0 {"scheduled_arrival"} else {"request_start"},
            "warmup_writes":bench_config.warmup_writes,"raft_config":bench_config.raft_config,"resolved_raft_config":resolved_config,
            "write_rate":write_rate,"append_delay_us":bench_config.append_delay_us,"flush_delay_us":bench_config.flush_delay_us,
            "network_delay_us":bench_config.network_delay_us,"replication_read_delay_us":bench_config.replication_read_delay_us,
            "apply_read_delay_us":bench_config.apply_read_delay_us,"apply_io_delay_us":bench_config.apply_io_delay_us,
            "apply_entry_delay_us":bench_config.apply_entry_delay_us,"arrival":bench_config.arrival,"measurement":bench_config.measurement});
        report["compiled_features"] = compiled_features();
        report["apply_pipeline"] = serde_json::to_value(apply_pipeline)?;
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

async fn collect_clients(handles: Vec<arrival::Task>) -> anyhow::Result<Vec<Samples>> {
    let mut groups = Vec::with_capacity(handles.len());
    for handle in handles {
        let samples = handle.await??;
        groups.push(*samples);
    }
    Ok(groups)
}

fn merge_samples(groups: Vec<Samples>, bounds: measurement::Bounds, trace_limit: usize) -> Samples {
    let mut summary = Samples::new(bounds, trace_limit);
    for samples in groups {
        summary.merge(samples);
    }
    summary
}

fn client_operations(total: u64, clients: u64, id: u64) -> u64 {
    assert!(clients > 0, "client count is positive");
    assert!(id < clients, "client ID is in range");
    total / clients + u64::from(id < total % clients)
}

async fn pace(epoch: Instant, operation: u64, write_rate: u64) -> Instant {
    let seconds = operation as f64 / write_rate as f64;
    let offset = Duration::from_secs_f64(seconds);
    let scheduled = epoch + offset;
    let deadline = tokio::time::Instant::from_std(scheduled);
    tokio::time::sleep_until(deadline).await;
    scheduled
}

#[cfg(test)]
fn percentile(samples: &[u64], percentile: usize) -> u64 {
    let nonempty = !samples.is_empty();
    assert!(nonempty, "latency samples must not be empty");
    let valid = (1..=100).contains(&percentile);
    assert!(valid, "percentile must be between 1 and 100");
    let rank = samples.len() * percentile;
    let rank = rank.div_ceil(100);
    samples[rank - 1]
}

fn compiled_features() -> serde_json::Value {
    serde_json::json!({
        "realistic-profiles":cfg!(feature = "realistic-profiles"),
        "runtime-stats":cfg!(feature = "runtime-stats"),
        "single-term-leader":cfg!(feature = "single-term-leader"),
        "bt":cfg!(feature = "bt"),
        "tokio-console":cfg!(feature = "tokio-console"),
        "flamegraph":cfg!(feature = "flamegraph"),
        "apply-pipeline":cfg!(feature = "apply-pipeline"),
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use clap::Parser;
    use openraft::ReadPolicy;

    use super::Args;
    use super::pace;
    use super::percentile;
    use super::validate;

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

    #[test]
    fn pure_reads_need_no_write_clients() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "bench",
            "--clients",
            "0",
            "--operations",
            "0",
            "--read-clients",
            "4",
            "--read-operations",
            "100",
        ])?;
        validate(&args)?;
        assert_eq!(0, args.clients);
        assert_eq!(4, args.read_clients);
        assert_eq!(100, args.read_operations);
        Ok(())
    }

    #[test]
    fn read_counts_require_read_clients() -> anyhow::Result<()> {
        let args = Args::try_parse_from(["bench", "--read-operations", "100"])?;
        let result = validate(&args);
        let error = result.expect_err("read operations cannot run without read clients");
        let message = error.to_string();
        assert_eq!("read-operations requires read-clients", message);
        Ok(())
    }

    #[test]
    fn pure_reads_reject_unserved_writes() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "bench",
            "--clients",
            "0",
            "--read-clients",
            "4",
            "--read-operations",
            "100",
        ])?;
        let result = validate(&args);
        let error = result.expect_err("the write operations must be disabled explicitly");
        let message = error.to_string();
        assert_eq!("operations must be zero without write clients", message);
        Ok(())
    }

    #[test]
    fn idle_delay_and_arrival_schedule_share_the_deadline() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "bench",
            "--clients",
            "0",
            "--operations",
            "0",
            "--read-clients",
            "1",
            "--read-operations",
            "399",
            "--read-rate",
            "1",
            "--read-delay-ms",
            "1000",
        ])?;
        let result = validate(&args);
        let error = result.expect_err("the combined idle and arrival schedule reaches the timeout");
        let message = error.to_string();
        assert_eq!(
            "arrival schedule plus read delay exceeds the benchmark timeout",
            message
        );
        Ok(())
    }

    #[test]
    fn relaxed_read_requires_a_query() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "bench",
            "--read-clients",
            "1",
            "--read-operations",
            "1",
            "--read-relaxed",
        ])?;
        let error = validate(&args).expect_err("relaxed reads must include the state-machine query");
        assert_eq!("read-relaxed requires read-query", error.to_string());
        let args = Args::try_parse_from([
            "bench",
            "--read-clients",
            "1",
            "--read-operations",
            "1",
            "--read-relaxed",
            "--read-query",
        ])?;
        validate(&args)?;
        Ok(())
    }

    #[test]
    fn read_policy_defaults_to_lease_reads() -> anyhow::Result<()> {
        let read = ["bench", "--read-clients", "1", "--read-operations", "1"];
        let args = Args::try_parse_from(read)?;
        validate(&args)?;
        assert!(args.read_policy.is_none());
        let args = Args::try_parse_from(read.iter().copied().chain(["--read-policy", "read-index"]))?;
        validate(&args)?;
        let policy = ReadPolicy::from(args.read_policy.unwrap_or_default());
        assert_eq!(ReadPolicy::ReadIndex, policy);
        Ok(())
    }

    #[test]
    fn follower_reads_require_a_query_on_a_follower() -> anyhow::Result<()> {
        let read = ["bench", "--read-clients", "1", "--read-operations", "1"];
        let cases = [
            (vec!["--read-on-follower", "1"], "read-on-follower requires read-query"),
            (
                vec!["--read-on-follower", "0", "--read-query"],
                "read-on-follower must name a follower, 1..members",
            ),
            (
                vec!["--read-on-follower", "3", "--read-query"],
                "read-on-follower must name a follower, 1..members",
            ),
        ];
        for (flags, expected) in cases {
            let args = Args::try_parse_from(read.iter().copied().chain(flags))?;
            let error = validate(&args).expect_err("the reader must be a follower that queries");
            assert_eq!(expected, error.to_string());
        }
        let follower = ["--read-on-follower", "2", "--read-query"];
        let args = Args::try_parse_from(read.iter().copied().chain(follower))?;
        validate(&args)?;
        Ok(())
    }

    #[test]
    fn leader_changes_need_readers_followers_and_single_writes() -> anyhow::Result<()> {
        let transfer = ["bench", "--transfer-leader-after-ms", "100"];
        let read = ["--read-clients", "1", "--read-operations", "1"];
        let cases = [
            (vec![], "transfer-leader-after-ms requires read-clients"),
            (
                vec!["-m", "1"],
                "transfer-leader-after-ms requires more than one member",
            ),
            (vec!["-b", "2"], "transfer-leader-after-ms requires batch 1"),
            (
                vec!["--read-query", "--read-on-follower", "1"],
                "transfer-leader-after-ms cannot read on a follower",
            ),
        ];
        for (flags, expected) in cases {
            let reads = if flags.is_empty() { &[][..] } else { &read[..] };
            let args = Args::try_parse_from(transfer.iter().chain(reads).copied().chain(flags))?;
            let error = validate(&args).expect_err("the leader change needs its preconditions");
            assert_eq!(expected, error.to_string());
        }
        let args = Args::try_parse_from(transfer.iter().chain(&read).copied())?;
        validate(&args)?;
        Ok(())
    }

    #[test]
    fn read_mode_flags_require_read_clients() -> anyhow::Result<()> {
        let cases = [
            ("--read-await-ready", "read-await-ready requires read-clients"),
            ("--read-query", "read-query requires read-clients"),
            ("--read-relaxed", "read-relaxed requires read-query"),
            ("--allow-read-errors", "allow-read-errors requires read-clients"),
            ("--read-policy=read-index", "read-policy requires read-clients"),
        ];
        for (flag, expected) in cases {
            let args = Args::try_parse_from(["bench", flag])?;
            let result = validate(&args);
            let error = result.expect_err("read flags require an active read workload");
            let actual = error.to_string();
            assert_eq!(expected, actual);
        }
        Ok(())
    }
    #[test]
    fn complete_batches_are_distributed_without_truncating_the_remainder() {
        let actual: Vec<_> = (0..256).map(|id| super::client_operations(30000, 256, id)).collect();
        let mut expected = vec![118; 48];
        expected.extend(vec![117; 208]);
        assert_eq!(expected, actual);
        let actual: Vec<_> = (0..256).map(|id| super::client_operations(7500, 256, id) * 4).collect();
        let mut expected = vec![120; 76];
        expected.extend(vec![116; 180]);
        assert_eq!(expected, actual);
    }

    #[test]
    fn a_measurement_that_rounds_to_zero_is_rejected() -> anyhow::Result<()> {
        let args = Args::try_parse_from(["bench", "--measurement-seconds", "0.000000000001"])?;
        let result = validate(&args);
        let error = result.expect_err("a measurement window must contain at least one nanosecond");
        assert_eq!("measurement-seconds rounds to zero nanoseconds", error.to_string());
        Ok(())
    }

    #[test]
    fn timed_idle_and_transfer_must_fit_the_window() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "bench",
            "--measurement-seconds",
            "1",
            "--warmup-seconds",
            "0",
            "--drain-seconds",
            "0",
            "--read-clients",
            "1",
            "--transfer-leader-after-ms",
            "600000",
        ])?;
        let error = validate(&args).expect_err("a transfer must not outlive the measurement");
        assert_eq!(
            "leadership transfer must start before measurement ends",
            error.to_string()
        );
        Ok(())
    }

    #[test]
    fn timed_measurement_enables_measured_arrival_controls() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "bench",
            "--measurement-seconds",
            "1",
            "--open-loop",
            "--write-rate",
            "1000",
            "--arrival-noop",
            "--phase-sample-every",
            "1",
        ])?;
        let actual = validate(&args).map_err(|error| error.to_string());
        assert_eq!(Ok(()), actual);
        Ok(())
    }

    #[test]
    fn pipeline_flag_is_available_only_with_its_feature() {
        let parsed = Args::try_parse_from(["bench", "--measure-apply-pipeline"]);
        #[cfg(feature = "apply-pipeline")]
        {
            let args = parsed.unwrap();
            let enabled = args.measurement.pipeline_enabled();
            assert!(enabled);
        }
        #[cfg(not(feature = "apply-pipeline"))]
        {
            let error = parsed.unwrap_err();
            let kind = error.kind();
            assert_eq!(clap::error::ErrorKind::UnknownArgument, kind);
        }
    }
}
