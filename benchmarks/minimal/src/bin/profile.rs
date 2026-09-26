//! Measure startup memory, write allocations, request latency, and snapshot costs.
//! Run with `cargo run --release --bin profile -- --help`; disable stats with
//! `--no-default-features`.

#[path = "profile/allocator.rs"]
mod allocator;
#[path = "profile/measurement.rs"]
mod measurement;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use anyhow::ensure;
use bench_minimal::network::BenchRaft;
use bench_minimal::network::Router;
use bench_minimal::store::ClientRequest;
use clap::Parser;
use futures::TryStreamExt;
use measurement::Measurement;
use openraft::Config;
use openraft::SnapshotPolicy;

const WARMUP_WRITES: usize = 1024;
const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Parser)]
struct Args {
    /// Number of concurrent write clients.
    #[arg(long, default_value_t = 64)]
    clients: usize,
    /// Total number of log entries to commit.
    #[arg(long, default_value_t = 262_144)]
    operations: usize,
    /// Entries in each timed write request.
    #[arg(long, default_value_t = 1)]
    batch: usize,
    /// Number of Raft members: 1, 3, or 5.
    #[arg(long, default_value_t = 3)]
    members: u64,
    /// Number of snapshots to build after the write workload.
    #[arg(long, default_value_t = 0)]
    snapshots: usize,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    validate(&args)?;
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.worker_threads(4);
    builder.enable_all();
    let runtime = builder.build()?;
    runtime.block_on(profile(&args))
}

fn validate(args: &Args) -> anyhow::Result<()> {
    ensure!(args.clients > 0, "clients must be positive");
    ensure!(args.batch > 0, "batch must be positive");
    let valid_members = [1, 3, 5].contains(&args.members);
    ensure!(valid_members, "members must be 1, 3, or 5");
    let group = args.clients.checked_mul(args.batch);
    let group = group.ok_or_else(|| anyhow::anyhow!("clients * batch overflows"))?;
    ensure!(args.operations >= group, "operations must cover every client");
    let complete_batches = args.operations.is_multiple_of(group);
    ensure!(complete_batches, "operations must be divisible by clients * batch");
    Ok(())
}

async fn profile(args: &Args) -> anyhow::Result<()> {
    let startup = Measurement::start();
    let (router, leader) = create_cluster(args).await?;
    tokio::task::yield_now().await;
    startup.report("startup", args, None, 0);
    for _ in 0..WARMUP_WRITES {
        leader.client_write(ClientRequest {}).await?;
    }
    profile_writes(args, &leader).await?;
    profile_snapshots(args, &leader).await?;
    let rafts = {
        let table = router.table.lock().unwrap();
        let values = table.values().cloned();
        values.collect::<Vec<_>>()
    };
    for raft in rafts {
        raft.shutdown().await?;
    }
    Ok(())
}

async fn create_cluster(args: &Args) -> anyhow::Result<(Router, BenchRaft)> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        snapshot_policy: SnapshotPolicy::Never,
        ..Default::default()
    };
    let config = config.validate()?;
    let config = Arc::new(config);
    let members: BTreeSet<_> = (0..args.members).collect();
    let mut router = Router::new();
    router.new_cluster(config, members).await?;
    let leader = router.get_raft(0);
    Ok((router, leader))
}

async fn profile_writes(args: &Args, leader: &BenchRaft) -> anyhow::Result<()> {
    let writes = Measurement::start();
    let mut handles = Vec::with_capacity(args.clients);
    for _ in 0..args.clients {
        let raft = leader.clone();
        let client_args = args.clone();
        let handle = tokio::spawn(write_client(raft, client_args));
        handles.push(handle);
    }
    let mut latencies = Vec::with_capacity(args.operations / args.batch);
    for handle in handles {
        let result = handle.await?;
        let samples = result?;
        latencies.extend(samples);
    }
    writes.report("writes", args, Some(&mut latencies), args.operations);
    Ok(())
}

async fn write_client(raft: BenchRaft, args: Args) -> anyhow::Result<Vec<u64>> {
    let batches = args.operations / args.clients / args.batch;
    let mut latencies = Vec::with_capacity(batches);
    for _ in 0..batches {
        let started = Instant::now();
        write_batch(&raft, args.batch).await?;
        let elapsed = started.elapsed();
        let nanos = elapsed.as_nanos();
        let nanos = u64::try_from(nanos)?;
        latencies.push(nanos);
    }
    Ok(latencies)
}

async fn write_batch(raft: &BenchRaft, batch: usize) -> anyhow::Result<()> {
    if batch == 1 {
        raft.client_write(ClientRequest {}).await?;
        return Ok(());
    }
    let requests: Vec<_> = (0..batch).map(|_| ClientRequest {}).collect();
    let mut responses = raft.client_write_many(requests).await?;
    while let Some(response) = responses.try_next().await? {
        response?;
    }
    Ok(())
}

async fn profile_snapshots(args: &Args, leader: &BenchRaft) -> anyhow::Result<()> {
    if args.snapshots == 0 {
        return Ok(());
    }
    let snapshots = Measurement::start();
    for _ in 0..args.snapshots {
        let response = leader.client_write(ClientRequest {}).await?;
        let trigger = leader.trigger();
        trigger.snapshot().await?;
        let wait = leader.wait(Some(WAIT_TIMEOUT));
        wait.snapshot(response.log_id, "snapshot built").await?;
    }
    snapshots.report("snapshots", args, None, args.snapshots);
    Ok(())
}
