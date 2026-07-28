//! Tick-based fuzzer for OpenRaft turmoil tests.
//!
//! Modes:
//!   Fuzz mode:      fuzz --seed <SEED> --max-steps <N> [--crash-file <PATH>]
//!   Reproduce mode: fuzz --reproduce <ITERATION_SEED> --max-steps <N> [--crash-file <PATH>]

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::fs;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use clap::Parser;
use openraft::async_runtime::WatchReceiver;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use serde::Serialize;
use tests_turmoil::cluster::ClusterState;
use tests_turmoil::cluster::bounce_node;
use tests_turmoil::cluster::crash_node;
use tests_turmoil::cluster::host_name;
use tests_turmoil::cluster::register_node_storage;
use tests_turmoil::cluster::spawn_host;
use tests_turmoil::invariants::InvariantChecker;
use tests_turmoil::liveness;
use tests_turmoil::oracle::ClientHistory;
use tests_turmoil::store::StateMachineData;
use tests_turmoil::typ::*;

/// Liveness phase A: max ticks for the healed cluster to fully converge.
const LIVENESS_CONVERGE_DEADLINE: u64 = 30_000;
/// Liveness phase B: max ticks to serve the required writes and reads.
const SERVICE_CHECK_DEADLINE: u64 = 20_000;
/// Liveness phase B: writes and reads that must each succeed after healing.
const SERVICE_OPS: u64 = 10;
/// How often (in ticks) convergence is evaluated during liveness phase A.
const CONVERGE_CHECK_EVERY: u64 = 200;

#[derive(Debug, Clone, Serialize)]
struct DerivedConfig {
    num_initial_nodes: usize,
    max_potential_nodes: u64,
    fail_rate: f64,
    heartbeat_interval: u64,
    election_timeout_min: u64,
    election_timeout_max: u64,
    enable_chaos: bool,
    restart_chance: f64,
    chaos_interval: u64,
    membership_interval: u64,
    /// Take a snapshot after this many new committed logs.
    snapshot_logs_threshold: u64,
    /// Keep at most this many applied logs around after a snapshot.
    /// Smaller = more aggressive purging = snapshot install is more likely
    /// when a lagging follower returns.
    max_in_snapshot_log_to_keep: u64,
    /// Switch a follower from log-shipping to snapshot install when it
    /// falls this far behind the leader. Must exceed `snapshot_logs_threshold`.
    replication_lag_threshold: u64,
    /// Probability a crash becomes a long outage instead of a short bounce.
    /// Long outages let the follower fall far enough behind to require
    /// snapshot install when it rejoins.
    long_outage_chance: f64,
    /// Minimum downtime (in ticks) for a long outage.
    long_outage_min_ticks: u64,
    /// Maximum downtime (in ticks) for a long outage.
    long_outage_max_ticks: u64,
    enable_leader_restore: bool,
    /// Run the real Pre-Vote protocol (a wire RPC in this harness).
    enable_pre_vote: bool,
    /// Number of distinct keys the workload writes and reads.
    key_space: u64,
    /// Base per-message latency floor for every link.
    min_message_latency_ms: u64,
    /// Base per-message latency ceiling for every link.
    max_message_latency_ms: u64,
    /// Schedule random trigger ops (elect / snapshot / purge / transfer-leader).
    enable_trigger_ops: bool,
    trigger_interval: u64,
    /// Chance per workload firing to enter a quiet window (no writes for a
    /// while). Quiescence is a precondition for some liveness bugs.
    quiet_window_chance: f64,
    /// Stop writes this many ticks before the safety phase ends (0 = never).
    /// Traps armed late in the safety phase (e.g. a fully-purged leader with
    /// an unestablished follower) survive into the liveness phase only if no
    /// write undoes them; a traffic tail-off makes that window real.
    pre_liveness_quiet_ticks: u64,
}

impl DerivedConfig {
    fn from_seed(seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let heartbeat_interval = 50 + rng.gen_range(0..100);
        let election_timeout_min = heartbeat_interval * rng.gen_range(2..4);
        let snapshot_logs_threshold = rng.gen_range(100..=250);
        let num_initial_nodes = 3 + rng.gen_range(0..3);
        let fail_rate = rng.gen_range(0.0..0.002);
        let election_timeout_max = election_timeout_min + rng.gen_range(100..500);
        let enable_chaos = rng.gen_bool(0.8);
        let enable_leader_restore = rng.gen_bool(0.5);
        let enable_pre_vote = rng.gen_bool(0.5);
        // An aggressive zero-keep purge policy makes "fully purged log" states
        // reachable, which some replication paths only hit there.
        let max_in_snapshot_log_to_keep = if rng.gen_bool(0.2) { 0 } else { rng.gen_range(30..=80) };
        Self {
            num_initial_nodes,
            max_potential_nodes: 10,
            fail_rate: if enable_chaos { fail_rate } else { 0.0 },
            heartbeat_interval,
            election_timeout_min,
            election_timeout_max,
            enable_chaos,                              // 80% chance
            restart_chance: rng.gen_range(0.01..0.05), // 1-5%
            chaos_interval: rng.gen_range(2000..5000),
            membership_interval: rng.gen_range(10000..25000),
            snapshot_logs_threshold,
            max_in_snapshot_log_to_keep,
            replication_lag_threshold: snapshot_logs_threshold * 2,
            long_outage_chance: rng.gen_range(0.1..=0.3), // 10-30% of crashes
            long_outage_min_ticks: 5000,
            long_outage_max_ticks: 15000,
            enable_leader_restore,
            enable_pre_vote,
            key_space: rng.gen_range(10..=500),
            min_message_latency_ms: rng.gen_range(0..=2),
            max_message_latency_ms: rng.gen_range(5..=50),
            enable_trigger_ops: rng.gen_bool(0.7),
            trigger_interval: rng.gen_range(2000..6000),
            quiet_window_chance: rng.gen_range(0.02..=0.06),
            pre_liveness_quiet_ticks: if rng.gen_bool(0.25) {
                rng.gen_range(2000..=8000)
            } else {
                0
            },
        }
    }
}

impl std::fmt::Display for DerivedConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Derived config:")?;
        writeln!(f, "  num_initial_nodes: {}", self.num_initial_nodes)?;
        writeln!(f, "  max_potential_nodes: {}", self.max_potential_nodes)?;
        writeln!(f, "  fail_rate: {:.4}", self.fail_rate)?;
        writeln!(f, "  heartbeat_interval: {}ms", self.heartbeat_interval)?;
        writeln!(f, "  election_timeout_min: {}ms", self.election_timeout_min)?;
        writeln!(f, "  election_timeout_max: {}ms", self.election_timeout_max)?;
        writeln!(f, "  enable_chaos: {}", self.enable_chaos)?;
        writeln!(f, "  restart_chance: {:.4}", self.restart_chance)?;
        writeln!(f, "  chaos_interval: {}", self.chaos_interval)?;
        writeln!(f, "  membership_interval: {}", self.membership_interval)?;
        writeln!(f, "  snapshot_logs_threshold: {}", self.snapshot_logs_threshold)?;
        writeln!(f, "  max_in_snapshot_log_to_keep: {}", self.max_in_snapshot_log_to_keep)?;
        writeln!(f, "  replication_lag_threshold: {}", self.replication_lag_threshold)?;
        writeln!(f, "  long_outage_chance: {:.4}", self.long_outage_chance)?;
        writeln!(f, "  long_outage_min_ticks: {}", self.long_outage_min_ticks)?;
        writeln!(f, "  long_outage_max_ticks: {}", self.long_outage_max_ticks)?;
        writeln!(f, "  enable_leader_restore: {}", self.enable_leader_restore)?;
        writeln!(f, "  enable_pre_vote: {}", self.enable_pre_vote)?;
        writeln!(f, "  key_space: {}", self.key_space)?;
        writeln!(f, "  min_message_latency_ms: {}", self.min_message_latency_ms)?;
        writeln!(f, "  max_message_latency_ms: {}", self.max_message_latency_ms)?;
        writeln!(f, "  enable_trigger_ops: {}", self.enable_trigger_ops)?;
        writeln!(f, "  trigger_interval: {}", self.trigger_interval)?;
        writeln!(f, "  quiet_window_chance: {:.4}", self.quiet_window_chance)?;
        write!(f, "  pre_liveness_quiet_ticks: {}", self.pre_liveness_quiet_ticks)
    }
}

/// OpenRaft Turmoil Fuzzer (tick-based)
#[derive(Parser)]
#[command(name = "fuzz")]
struct FuzzConfig {
    /// Base RNG seed for fuzzing [default: random]
    #[arg(short, long)]
    seed: Option<u64>,

    /// Exact iteration seed to reproduce
    #[arg(short, long)]
    reproduce: Option<u64>,

    /// Max steps per iteration
    #[arg(long, alias = "steps", default_value = "100000")]
    max_steps: u64,

    /// Number of iterations (0=forever)
    #[arg(short, long, default_value = "100")]
    iterations: u64,

    /// Where to write crash info
    #[arg(long)]
    crash_file: Option<String>,
}

/// Quiet default so fuzz throughput is not bounded by log formatting.
/// `RUST_LOG`, when set, replaces it entirely — e.g. `RUST_LOG=openraft=trace`
/// when reproducing a failure.
const DEFAULT_LOG_FILTER: &str = "openraft=error,warn";

fn main() {
    // Logs go to stderr; the fuzzer's own status output stays on stdout.
    let log_filter = std::env::var("RUST_LOG").unwrap_or_else(|_| DEFAULT_LOG_FILTER.to_string());
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::new(log_filter))
        .init();

    let config = FuzzConfig::parse();

    let (base_seed, iterations) = if let Some(seed) = config.reproduce {
        println!("=== OpenRaft Fuzzer REPRODUCE MODE ===");
        println!("Seed: {seed}, Max steps: {}", config.max_steps);
        println!(
            "\n{}\n======================================\n",
            DerivedConfig::from_seed(seed)
        );
        (seed, 1)
    } else {
        let seed = config.seed.unwrap_or_else(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
        });
        println!("=== OpenRaft Turmoil Fuzzer (tick-based) ===");
        println!(
            "Seed: {seed}, Max steps/iter: {}, Iterations: {} (0=forever)",
            config.max_steps, config.iterations
        );
        println!("============================================\n");
        (seed, config.iterations)
    };

    run_fuzz_loop(base_seed, config.max_steps, iterations, config.crash_file);
}

fn setup_ctrlc() -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        println!("\n\nInterrupted!");
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");
    running
}

struct FuzzResult {
    steps_completed: u64,
    invariant_checks: u64,
    violations: Vec<String>,
}

impl std::fmt::Display for FuzzResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Steps: {}, Checks: {}", self.steps_completed, self.invariant_checks)?;
        if !self.violations.is_empty() {
            writeln!(f, "\nViolations:")?;
            for v in &self.violations {
                writeln!(f, "  - {v}")?;
            }
        }
        Ok(())
    }
}

fn report_failure(iteration_seed: u64, max_steps: u64, result: &FuzzResult, derived: &DerivedConfig) {
    println!("\n{derived}\n\n{result}");
    println!("REPRODUCE WITH:");
    println!("  cargo run --bin fuzz -- --reproduce {iteration_seed} --max-steps {max_steps}");
}

fn write_crash_file(
    path: &str,
    base_seed: u64,
    iteration: u64,
    iteration_seed: u64,
    max_steps: u64,
    result: &FuzzResult,
    config: &DerivedConfig,
) {
    let crash_info = serde_json::json!({
        "base_seed": base_seed,
        "iteration": iteration,
        "iteration_seed": iteration_seed,
        "max_steps": max_steps,
        "steps_completed": result.steps_completed,
        "violation": result.violations.first(),
        "config": serde_json::to_value(config).unwrap(),
        "reproduce": {
            "command": format!(
                "cargo run --bin fuzz -- --reproduce {} --max-steps {} --crash-file {}",
                iteration_seed, max_steps, path
            ),
            "iteration_seed": iteration_seed,
            "max_steps": max_steps
        }
    });
    if let Err(e) = fs::write(path, serde_json::to_string_pretty(&crash_info).unwrap()) {
        eprintln!("Failed to write crash file: {e}");
    }
}

fn run_fuzz_loop(base_seed: u64, max_steps: u64, iterations: u64, crash_file: Option<String>) {
    let running = setup_ctrlc();
    let mut iteration = 0u64;
    let mut total_steps = 0u64;
    let mut total_checks = 0u64;

    loop {
        if !running.load(Ordering::Relaxed) {
            break;
        }
        if iterations > 0 && iteration >= iterations {
            break;
        }

        let iteration_seed = base_seed.wrapping_add(iteration);
        let derived = DerivedConfig::from_seed(iteration_seed);

        println!(
            "--- Iteration {} (seed: {}, nodes: {}, fail_rate: {:.2}%, chaos: {}, leader_restore: {}) ---",
            iteration + 1,
            iteration_seed,
            derived.num_initial_nodes,
            derived.fail_rate * 100.0,
            derived.enable_chaos,
            derived.enable_leader_restore,
        );

        // A panic escaping the simulation (e.g. out of a host's `Drop` or the
        // harness itself) must fail the run, not abort the fuzz loop silently.
        // Panics *inside* host software surface as step errors and are
        // handled in `step_tick`.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_single_iteration(iteration_seed, &derived, max_steps, running.clone())
        }))
        .unwrap_or_else(|payload| {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "non-string panic payload".to_string()
            };
            FuzzResult {
                steps_completed: 0,
                invariant_checks: 0,
                violations: vec![format!("Panic: {msg}")],
            }
        });
        total_steps += result.steps_completed;
        total_checks += result.invariant_checks;

        if !result.violations.is_empty() {
            println!(
                "\n=== FAILED at iteration {} (seed: {iteration_seed}) ===",
                iteration + 1
            );
            report_failure(iteration_seed, max_steps, &result, &derived);
            if let Some(path) = &crash_file {
                write_crash_file(path, base_seed, iteration, iteration_seed, max_steps, &result, &derived);
            }
            std::process::exit(1);
        }

        iteration += 1;
    }

    println!("\n=== Results ===");
    println!("Iterations: {iteration}, Steps: {total_steps}, Checks: {total_checks}");
    println!("Status: PASSED");
}

struct NetworkChaos {
    rng: StdRng,
    next_step: u64,
}

impl NetworkChaos {
    fn new(seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let next_step = rng.gen_range(1000..5000);
        Self { rng, next_step }
    }

    fn maybe_apply(
        &mut self,
        steps: u64,
        sim: &mut turmoil::Sim,
        max_nodes: u64,
        derived: &DerivedConfig,
        cluster_state: &Arc<Mutex<ClusterState>>,
    ) {
        if steps < self.next_step {
            return;
        }

        apply_network_chaos(sim, &mut self.rng, max_nodes, derived, cluster_state);
        self.next_step = steps + self.rng.gen_range(1000..5000);
    }
}

/// One operation for the in-simulation driver client to execute.
enum DriverOp {
    /// A tracked client write; its outcome is recorded in [`ClientHistory`].
    Write { raft: Arc<Raft>, req: Request },
    /// A Raft trigger API call; failures are expected and ignored.
    Trigger { raft: Arc<Raft>, kind: TriggerKind },
}

#[derive(Debug)]
enum TriggerKind {
    Elect {
        pre_vote: bool,
    },
    Snapshot,
    PurgeLog {
        upto: u64,
    },
    TransferLeader {
        to: NodeId,
    },
    /// Snapshot then purge up to that snapshot: log compaction in one stroke.
    /// On an idle leader this purges the log up to its very tip, exercising
    /// replication paths that must fall back to snapshots (#1828 class).
    Compact,
}

type DriverQueue = Arc<Mutex<VecDeque<DriverOp>>>;
type ReadQueue = Arc<Mutex<VecDeque<String>>>;

struct WorkloadSchedule {
    rng: StdRng,
    next_step: u64,
    next_serial: u64,
    key_space: u64,
    quiet_chance: f64,
    quiet_until: u64,
    /// No scheduled writes at or after this step (pre-liveness tail-off).
    hard_quiet_from: u64,
    attempts: Arc<AtomicU64>,
}

struct MembershipTask {
    done: Arc<AtomicBool>,
    success: Arc<AtomicBool>,
}

struct MembershipInflight {
    task: MembershipTask,
    new_voters: BTreeSet<NodeId>,
}

/// One desired membership change. A failed attempt keeps the plan and retries
/// the *same* target after a cooldown, so heavy chaos cannot permanently
/// starve the joint-consensus success path by re-rolling a new change each time.
#[derive(Clone)]
struct MembershipPlan {
    desired: BTreeSet<NodeId>,
    learners_to_add: Vec<(NodeId, Node)>,
    /// Passed to `change_membership`: removed voters stay as learners
    /// (demotion) instead of being dropped from the config.
    retain: bool,
}

#[derive(Default)]
struct MembershipStats {
    attempts: u64,
    applied: u64,
    failed: u64,
}

impl std::fmt::Display for MembershipStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "membership attempts/applied/failed={}/{}/{}",
            self.attempts, self.applied, self.failed
        )
    }
}

impl WorkloadSchedule {
    fn new(seed: u64, key_space: u64, quiet_chance: f64, hard_quiet_from: u64, attempts: Arc<AtomicU64>) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let next_step = rng.gen_range(10..50);
        Self {
            rng,
            next_step,
            next_serial: 0,
            key_space,
            quiet_chance,
            quiet_until: 0,
            hard_quiet_from,
            attempts,
        }
    }

    /// Build the next write request. Every request gets a globally unique
    /// serial; the value embeds the serial so any observed value maps back to
    /// exactly one attempt.
    fn build_request(&mut self) -> Request {
        let serial = self.next_serial;
        self.next_serial += 1;
        self.attempts.store(self.next_serial, Ordering::SeqCst);
        Request {
            client_id: "workload".to_string(),
            serial,
            key: format!("key-{}", self.rng.gen_range(0..self.key_space)),
            value: format!("value-{}-{}", serial, self.rng.r#gen::<u32>()),
        }
    }

    fn maybe_enqueue(
        &mut self,
        steps: u64,
        queue: &DriverQueue,
        cluster_state: &Arc<Mutex<ClusterState>>,
        paused: &Arc<AtomicBool>,
        history: &Arc<Mutex<ClientHistory>>,
    ) {
        if paused.load(Ordering::SeqCst) || steps < self.next_step || steps < self.quiet_until {
            return;
        }
        if steps >= self.hard_quiet_from {
            return;
        }

        if self.rng.gen_bool(self.quiet_chance) {
            self.quiet_until = steps + self.rng.gen_range(1000..=8000);
            println!("WORKLOAD: quiet window until step {}", self.quiet_until);
            self.next_step = steps + self.rng.gen_range(10..50);
            return;
        }

        if let Some(raft) = cluster_state.lock().unwrap().find_leader() {
            let req = self.build_request();
            enqueue_write(queue, history, raft, req);
        }

        self.next_step = steps + self.rng.gen_range(10..50);
    }
}

/// Record a write attempt and hand it to the driver. Recording happens before
/// the request can reach any node, so every observable value is a known attempt.
fn enqueue_write(queue: &DriverQueue, history: &Arc<Mutex<ClientHistory>>, raft: Arc<Raft>, req: Request) {
    history.lock().unwrap().record_write_attempt(req.serial, req.key.clone(), req.value.clone());
    queue.lock().unwrap().push_back(DriverOp::Write { raft, req });
}

/// Schedules linearizable reads of random keys.
struct ReadSchedule {
    rng: StdRng,
    next_step: u64,
    key_space: u64,
}

impl ReadSchedule {
    fn new(seed: u64, key_space: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let next_step = rng.gen_range(20..80);
        Self {
            rng,
            next_step,
            key_space,
        }
    }

    fn pick_key(&mut self) -> String {
        format!("key-{}", self.rng.gen_range(0..self.key_space))
    }

    /// Reads keep flowing during membership pauses and workload quiet windows:
    /// they propose nothing, so they never mask quiescence-dependent bugs.
    fn maybe_enqueue(&mut self, steps: u64, queue: &ReadQueue) {
        if steps < self.next_step {
            return;
        }
        let key = self.pick_key();
        queue.lock().unwrap().push_back(key);
        self.next_step = steps + self.rng.gen_range(20..80);
    }
}

/// Schedules random Raft trigger operations: forced elections, snapshot
/// builds, log purges and leader transfers (including to learners and
/// non-members, which a correct implementation must reject, not crash on).
struct TriggerSchedule {
    rng: StdRng,
    next_step: u64,
    interval: u64,
}

impl TriggerSchedule {
    fn new(seed: u64, interval: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let next_step = interval + rng.gen_range(0..interval);
        Self {
            rng,
            next_step,
            interval,
        }
    }

    fn maybe_enqueue(
        &mut self,
        steps: u64,
        queue: &DriverQueue,
        max_nodes: u64,
        cluster_state: &Arc<Mutex<ClusterState>>,
    ) {
        if steps < self.next_step {
            return;
        }
        self.next_step = steps + self.interval + self.rng.gen_range(0..self.interval);

        let state = cluster_state.lock().unwrap();
        let op = match self.rng.gen_range(0..5) {
            0 => {
                let node = self.rng.gen_range(1..=max_nodes);
                let pre_vote = self.rng.r#gen::<bool>();
                state.get_raft(node).map(|raft| (node, raft, TriggerKind::Elect { pre_vote }))
            }
            1 => {
                let node = self.rng.gen_range(1..=max_nodes);
                state.get_raft(node).map(|raft| (node, raft, TriggerKind::Snapshot))
            }
            2 => {
                let node = self.rng.gen_range(1..=max_nodes);
                state.get_raft(node).and_then(|raft| {
                    let snapshot = raft.metrics().borrow_watched().snapshot;
                    snapshot.map(|log_id| (node, raft, TriggerKind::PurgeLog { upto: log_id.index }))
                })
            }
            3 => {
                let to = self.rng.gen_range(1..=max_nodes);
                state.find_leader_entry().map(|(id, raft)| (id, raft, TriggerKind::TransferLeader { to }))
            }
            4 => {
                // Compaction matters most on the leader, whose purged log
                // forces replication to fall back to snapshots; hit a random
                // node half of the time anyway.
                if self.rng.r#gen::<bool>() {
                    state.find_leader_entry().map(|(id, raft)| (id, raft, TriggerKind::Compact))
                } else {
                    let node = self.rng.gen_range(1..=max_nodes);
                    state.get_raft(node).map(|raft| (node, raft, TriggerKind::Compact))
                }
            }
            _ => unreachable!(),
        };
        drop(state);

        if let Some((node, raft, kind)) = op {
            println!("TRIGGER: {kind:?} on node {node} at step {steps}");
            queue.lock().unwrap().push_back(DriverOp::Trigger { raft, kind });
        }
    }
}

/// Restore every link to its seed-derived baseline: latency band back to
/// (min, max), per-link loss back to `fail_rate` (or zero when healing).
fn restore_link_quality(sim: &mut turmoil::Sim, max_nodes: u64, derived: &DerivedConfig, heal: bool) {
    let fail_rate = if heal { 0.0 } else { derived.fail_rate };
    for i in 1..=max_nodes {
        for j in (i + 1)..=max_nodes {
            let (a, b) = (host_name(i), host_name(j));
            // `set_link_latency` collapses the band to min=max; widening the
            // max afterwards restores the (min, max) jitter band.
            sim.set_link_latency(
                a.as_str(),
                b.as_str(),
                Duration::from_millis(derived.min_message_latency_ms),
            );
            sim.set_link_max_message_latency(
                a.as_str(),
                b.as_str(),
                Duration::from_millis(derived.max_message_latency_ms),
            );
            sim.set_link_fail_rate(a.as_str(), b.as_str(), fail_rate);
        }
    }
    if heal {
        sim.set_fail_rate(0.0);
    }
}

/// Remove every fault: partitions, held messages, latency spikes, loss.
/// After this the network is perfect; the cluster must fully heal on its own.
fn heal_network(sim: &mut turmoil::Sim, max_nodes: u64, derived: &DerivedConfig) {
    for i in 1..=max_nodes {
        for j in 1..=max_nodes {
            if i != j {
                sim.release(host_name(i), host_name(j));
            }
        }
    }
    for i in 1..=max_nodes {
        for j in (i + 1)..=max_nodes {
            sim.repair(host_name(i), host_name(j));
        }
    }
    restore_link_quality(sim, max_nodes, derived, true);
}

fn apply_network_chaos(
    sim: &mut turmoil::Sim,
    rng: &mut StdRng,
    max_nodes: u64,
    derived: &DerivedConfig,
    cluster_state: &Arc<Mutex<ClusterState>>,
) {
    match rng.gen_range(0..9) {
        0 => {
            // Single-node isolation: one node unreachable from all others.
            // Does not threaten quorum in 3+ clusters on its own.
            let victim = rng.gen_range(1..=max_nodes);
            for i in 1..=max_nodes {
                if i != victim {
                    sim.partition(host_name(victim), host_name(i));
                }
            }
        }
        1 => {
            // Global repair: undo all partitions.
            for i in 1..=max_nodes {
                for j in (i + 1)..=max_nodes {
                    sim.repair(host_name(i), host_name(j));
                }
            }
        }
        2 => {
            // One-way hold on a single pair (delivers later).
            let a = rng.gen_range(1..=max_nodes);
            let mut b = rng.gen_range(1..=max_nodes);
            while b == a {
                b = rng.gen_range(1..=max_nodes);
            }
            sim.hold(host_name(a), host_name(b));
        }
        3 => {
            // Global release: flush all held messages.
            for i in 1..=max_nodes {
                for j in 1..=max_nodes {
                    if i != j {
                        sim.release(host_name(i), host_name(j));
                    }
                }
            }
        }
        4 => {
            // Minority partition: split the cluster into a minority side
            // (size 1..=max_nodes/2) vs the rest. The majority side
            // retains quorum; the minority cannot commit until repaired.
            let minority_size = rng.gen_range(1..=max_nodes / 2);
            let mut all: Vec<u64> = (1..=max_nodes).collect();
            all.shuffle(rng);
            let minority: BTreeSet<u64> = all.into_iter().take(minority_size as usize).collect();
            for &a in &minority {
                for b in 1..=max_nodes {
                    if !minority.contains(&b) {
                        sim.partition(host_name(a), host_name(b));
                    }
                }
            }
        }
        5 => {
            // Leader-in-minority partition: isolate the current leader
            // together with a random subset of peers on the minority side,
            // forcing the majority to re-elect while the old leader may
            // still believe it leads. This is where stale-leader bugs
            // and dueling-term scenarios live.
            let Some(leader_id) = cluster_state.lock().unwrap().find_leader_id() else {
                return;
            };
            let majority_needed = (max_nodes / 2) + 1;
            let extra = if max_nodes > majority_needed {
                rng.gen_range(0..=(max_nodes - majority_needed))
            } else {
                0
            };
            let mut others: Vec<u64> = (1..=max_nodes).filter(|n| *n != leader_id).collect();
            others.shuffle(rng);
            let mut minority: BTreeSet<u64> = others.into_iter().take(extra as usize).collect();
            minority.insert(leader_id);
            for &a in &minority {
                for b in 1..=max_nodes {
                    if !minority.contains(&b) {
                        sim.partition(host_name(a), host_name(b));
                    }
                }
            }
        }
        6 => {
            // Link latency spike (jitter injection): one link's latency
            // ceiling jumps far above the baseline, so messages on it can
            // arrive after messages sent later on other links. Out-of-order
            // follower acks are exactly the kind of interleaving that broke
            // the leader's commit-quorum bookkeeping upstream (#1802).
            let a = rng.gen_range(1..=max_nodes);
            let mut b = rng.gen_range(1..=max_nodes);
            while b == a {
                b = rng.gen_range(1..=max_nodes);
            }
            let spike_ms = rng.gen_range(100..=800);
            sim.set_link_max_message_latency(host_name(a), host_name(b), Duration::from_millis(spike_ms));
        }
        7 => {
            // Link loss: one link drops a large fraction of messages,
            // exercising retry/backoff paths without a full partition.
            let a = rng.gen_range(1..=max_nodes);
            let mut b = rng.gen_range(1..=max_nodes);
            while b == a {
                b = rng.gen_range(1..=max_nodes);
            }
            let loss = rng.gen_range(0.05..=0.4);
            sim.set_link_fail_rate(host_name(a), host_name(b), loss);
        }
        8 => {
            // Jitter repair: all links return to the baseline latency band
            // and loss rate (partitions/holds are repaired by actions 1 and 3).
            restore_link_quality(sim, max_nodes, derived, false);
        }
        _ => unreachable!(),
    }
}

async fn membership_change_once(
    raft: Arc<Raft>,
    learners_to_add: Vec<(NodeId, Node)>,
    new_set: BTreeSet<NodeId>,
    retain: bool,
    workload_paused: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    success: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    // openraft requires a node to be a learner before promotion to voter.
    for (id, node) in learners_to_add {
        let result = tokio::time::timeout(Duration::from_millis(1000), raft.add_learner(id, node, true)).await;
        if result.is_err() {
            println!("MEMBERSHIP-CLIENT: add_learner timed out for node {id}");
        }
    }

    println!("MEMBERSHIP-CLIENT: executing change to {new_set:?} (retain={retain})");
    match tokio::time::timeout(
        Duration::from_millis(5000),
        raft.change_membership(new_set.clone(), retain),
    )
    .await
    {
        Ok(Ok(_)) => {
            success.store(true, Ordering::SeqCst);
        }
        Ok(Err(e)) => {
            println!("MEMBERSHIP-CLIENT: change_membership failed: {e}");
        }
        Err(_) => {
            println!("MEMBERSHIP-CLIENT: change_membership timed out");
        }
    }

    workload_paused.store(false, Ordering::SeqCst);
    done.store(true, Ordering::SeqCst);
    Ok(())
}

/// Finalize an interrupted membership change: propose the joint config's goal
/// set as the uniform config. Used by the liveness phase only.
async fn finalize_joint_once(
    raft: Arc<Raft>,
    goal: BTreeSet<NodeId>,
    done: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("MEMBERSHIP-CLIENT: finalizing joint config to {goal:?}");
    match tokio::time::timeout(Duration::from_millis(5000), raft.change_membership(goal, false)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => println!("MEMBERSHIP-CLIENT: finalize failed: {e}"),
        Err(_) => println!("MEMBERSHIP-CLIENT: finalize timed out"),
    }
    done.store(true, Ordering::SeqCst);
    Ok(())
}

/// If the leader's effective config is joint and no finalize client is in
/// flight, spawn one proposing the joint's goal set as the uniform config.
///
/// A `change_membership` call interrupted by chaos (client timeout drops the
/// future) can leave a joint config behind — it may even surface mid-heal,
/// when an election replays an uncommitted config entry. openraft only
/// finalizes a joint config on the next `change_membership` call, so the
/// liveness phase acts as the operator and re-issues the change until the
/// config is uniform.
fn maybe_finalize_joint(
    sim: &mut turmoil::Sim,
    steps: u64,
    cluster_state: &Arc<Mutex<ClusterState>>,
    inflight: &mut Option<Arc<AtomicBool>>,
) {
    if inflight.as_ref().is_some_and(|d| !d.load(Ordering::SeqCst)) {
        return;
    }
    let Some((leader_id, raft)) = cluster_state.lock().unwrap().find_leader_entry() else {
        return;
    };
    let snapshots = cluster_state.lock().unwrap().get_all_full_snapshots();
    let Some(membership) = snapshots
        .iter()
        .find(|(id, _)| *id == leader_id)
        .map(|(_, s)| s.raft.membership_config.membership().clone())
    else {
        return;
    };
    let joint = membership.get_joint_config();
    if joint.len() <= 1 {
        return;
    }
    let goal = joint.last().expect("joint config has entries").clone();
    let done = Arc::new(AtomicBool::new(false));
    sim.client(
        format!("finalize-{steps}"),
        finalize_joint_once(raft, goal, done.clone()),
    );
    *inflight = Some(done);
}

fn schedule_membership_change(
    sim: &mut turmoil::Sim,
    steps: u64,
    cluster_state: Arc<Mutex<ClusterState>>,
    plan: &MembershipPlan,
    workload_paused: Arc<AtomicBool>,
) -> Option<MembershipTask> {
    let Some(raft) = cluster_state.lock().unwrap().find_leader() else {
        println!("MEMBERSHIP: No leader at step {steps}; skipping scheduled change");
        return None;
    };

    workload_paused.store(true, Ordering::SeqCst);

    let done = Arc::new(AtomicBool::new(false));
    let success = Arc::new(AtomicBool::new(false));
    sim.client(
        format!("membership-{steps}"),
        membership_change_once(
            raft,
            plan.learners_to_add.clone(),
            plan.desired.clone(),
            plan.retain,
            workload_paused,
            done.clone(),
            success.clone(),
        ),
    );
    Some(MembershipTask { done, success })
}

/// Pick the next random membership change: grow towards 7 voters, shrink
/// above 3, and reuse previously removed nodes so stale rejoin paths
/// (snapshot install onto an outdated member) stay covered.
fn new_membership_plan(
    rng: &mut StdRng,
    active_voters: &BTreeSet<NodeId>,
    potential_nodes: &BTreeMap<NodeId, Node>,
) -> Option<MembershipPlan> {
    let add = active_voters.len() < 3 || (active_voters.len() < 7 && rng.gen_bool(0.7));
    if add {
        let candidates: Vec<NodeId> =
            potential_nodes.keys().copied().filter(|id| !active_voters.contains(id)).collect();
        let joiner = *candidates.get(rng.gen_range(0..candidates.len().max(1)))?;
        let mut desired = active_voters.clone();
        desired.insert(joiner);
        Some(MembershipPlan {
            desired,
            learners_to_add: vec![(joiner, potential_nodes.get(&joiner).cloned()?)],
            retain: rng.r#gen::<bool>(),
        })
    } else if active_voters.len() > 3 {
        let voters: Vec<NodeId> = active_voters.iter().copied().collect();
        let victim = voters[rng.gen_range(0..voters.len())];
        let mut desired = active_voters.clone();
        desired.remove(&victim);
        Some(MembershipPlan {
            desired,
            learners_to_add: Vec::new(),
            retain: rng.r#gen::<bool>(),
        })
    } else {
        None
    }
}

/// Executes queued driver ops inside the simulation.
///
/// Writes are tracked: the ack (with its log id) or failure is recorded in the
/// shared [`ClientHistory`]. A timed-out write stays "unknown" — it may still
/// commit later, and the oracle never assumes otherwise.
/// Per-op tracing for determinism debugging: `FUZZ_DEBUG_OPS=1` prints every
/// write/read completion in execution order; two runs of the same seed must
/// produce identical streams.
fn dbg_ops() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("FUZZ_DEBUG_OPS").is_ok_and(|v| v == "1"))
}

async fn driver_loop(queue: DriverQueue, history: Arc<Mutex<ClientHistory>>) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let ops: Vec<_> = queue.lock().unwrap().drain(..).collect();
        for op in ops {
            match op {
                DriverOp::Write { raft, req } => {
                    let history = history.clone();
                    tokio::spawn(async move {
                        let serial = req.serial;
                        match tokio::time::timeout(Duration::from_millis(5000), raft.client_write(req)).await {
                            Ok(Ok(resp)) => {
                                if dbg_ops() {
                                    println!("DBG ack serial={serial} log={}", resp.log_id);
                                }
                                history.lock().unwrap().record_write_acked(serial, resp.log_id, resp.data.prev)
                            }
                            _ => {
                                if dbg_ops() {
                                    println!("DBG wfail serial={serial}");
                                }
                                history.lock().unwrap().record_write_failed(serial)
                            }
                        }
                    });
                }
                DriverOp::Trigger { raft, kind } => {
                    tokio::spawn(async move {
                        let timeout = Duration::from_millis(1000);
                        match kind {
                            TriggerKind::Elect { pre_vote } => {
                                let _ = tokio::time::timeout(timeout, raft.trigger().elect(pre_vote)).await;
                            }
                            TriggerKind::Snapshot => {
                                let _ = tokio::time::timeout(timeout, raft.trigger().snapshot()).await;
                            }
                            TriggerKind::PurgeLog { upto } => {
                                let _ = tokio::time::timeout(timeout, raft.trigger().purge_log(upto)).await;
                            }
                            TriggerKind::TransferLeader { to } => {
                                let _ = tokio::time::timeout(timeout, raft.trigger().transfer_leader(to)).await;
                            }
                            TriggerKind::Compact => {
                                let _ = tokio::time::timeout(timeout, async {
                                    if raft.trigger().snapshot().await.is_err() {
                                        return;
                                    }
                                    let snapshot = raft.metrics().borrow_watched().snapshot;
                                    if let Some(log_id) = snapshot {
                                        let _ = raft.trigger().purge_log(log_id.index).await;
                                    }
                                })
                                .await;
                            }
                        }
                    });
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// Executes linearizable reads sequentially: `ReadIndex` barrier on the
/// current leader, then a local state-machine read on that same node.
///
/// Sequential execution is what makes the per-key monotonic-read check sound.
async fn read_client_loop(
    queue: ReadQueue,
    cluster_state: Arc<Mutex<ClusterState>>,
    history: Arc<Mutex<ClientHistory>>,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let key = queue.lock().unwrap().pop_front();
        let Some(key) = key else {
            tokio::time::sleep(Duration::from_millis(1)).await;
            continue;
        };

        let leader = cluster_state.lock().unwrap().find_leader_entry();
        let Some((leader_id, raft)) = leader else {
            history.lock().unwrap().record_read_failed();
            continue;
        };
        let sm = cluster_state.lock().unwrap().get_state_machine(leader_id).expect("sm must be registered");

        // The read-your-writes floor must be captured before the read starts:
        // any ack recorded by now strictly precedes this read in real time.
        let floor = history.lock().unwrap().acked_floor(&key);

        match tokio::time::timeout(
            Duration::from_millis(2000),
            raft.ensure_linearizable(openraft::ReadPolicy::ReadIndex),
        )
        .await
        {
            Ok(Ok(barrier)) => {
                let observed = sm.get_key(&key);
                if dbg_ops() {
                    println!(
                        "DBG read key={key} observed={:?} barrier={:?}",
                        observed.as_ref().map(|m| m.log_id),
                        barrier
                    );
                }
                history.lock().unwrap().record_read(&key, observed.as_ref(), floor, barrier);
            }
            _ => {
                if dbg_ops() {
                    println!("DBG rfail key={key} leader=n{leader_id}");
                }
                history.lock().unwrap().record_read_failed()
            }
        }
    }
}

/// Advance the simulation by one tick.
///
/// Any error is a violation and fails the run:
///
/// - `Ok(true)` means all client hosts exited — the forever-running driver is gone, so the harness
///   itself is broken.
/// - `Err` covers host panics (turmoil surfaces them as step errors) and the simulation-duration
///   overrun, which is sized in the builder to cover both phases and thus unreachable in a healthy
///   run.
///
/// Swallowing these would mark a run PASSED after e.g. a node panicked —
/// panics are exactly the kind of bug this fuzzer must report (upstream #1805
/// was a leader-transfer panic).
fn step_tick(sim: &mut turmoil::Sim) -> Result<(), String> {
    match sim.step() {
        Ok(false) => Ok(()),
        Ok(true) => Err("HarnessFailure: all fuzz clients exited unexpectedly".to_string()),
        Err(e) => Err(format!("SimulationError: {e}")),
    }
}

/// Mutable per-iteration bookkeeping shared by the safety and liveness phases.
struct IterationState {
    steps: u64,
    invariant_checks: u64,
    violations: Vec<String>,
    invariants: InvariantChecker,
}

impl IterationState {
    fn new() -> Self {
        Self {
            steps: 0,
            invariant_checks: 0,
            violations: Vec::new(),
            invariants: InvariantChecker::default(),
        }
    }

    /// Advance the sim one tick and run every checker: server-side invariants
    /// and the client-observation oracle. Returns false when the iteration
    /// must stop (a violation was recorded).
    fn step_and_check(
        &mut self,
        sim: &mut turmoil::Sim,
        cluster_state: &Arc<Mutex<ClusterState>>,
        history: &Arc<Mutex<ClientHistory>>,
    ) -> bool {
        if let Err(msg) = step_tick(sim) {
            self.record_violation(format!("Step {}: {msg}", self.steps));
            return false;
        }
        self.steps += 1;

        let (snapshots, durable_logs) = {
            let state = cluster_state.lock().unwrap();
            (state.get_all_full_snapshots(), state.get_all_durable_log_ids())
        };
        self.invariant_checks += 1;
        let result = self.invariants.check_with_durable_logs(&snapshots, &durable_logs);
        for v in result.violations {
            self.record_violation(format!("Step {}: {v:?}", self.steps));
        }
        for v in history.lock().unwrap().drain_violations() {
            self.record_violation(format!("Step {}: {v}", self.steps));
        }
        self.violations.is_empty()
    }

    fn record_violation(&mut self, msg: String) {
        println!("VIOLATION: {msg}");
        self.violations.push(msg);
    }

    fn into_result(self) -> FuzzResult {
        FuzzResult {
            steps_completed: self.steps,
            invariant_checks: self.invariant_checks,
            violations: self.violations,
        }
    }
}

/// Everything the workload side of the fuzzer shares between phases.
struct WorkloadCtx {
    schedule: WorkloadSchedule,
    reads: ReadSchedule,
    driver_queue: DriverQueue,
    read_queue: ReadQueue,
    history: Arc<Mutex<ClientHistory>>,
    paused: Arc<AtomicBool>,
}

/// Print one line per node for liveness-failure diagnosis.
fn dump_cluster_state(cluster_state: &Arc<Mutex<ClusterState>>) {
    let snapshots = cluster_state.lock().unwrap().get_all_full_snapshots();
    println!("--- cluster state dump ---");
    for (id, s) in &snapshots {
        println!(
            "  n{id}: state={:?} vote={} last_log={:?} applied={:?} snapshot={:?} purged={:?} membership={}",
            s.raft.state,
            s.raft.vote,
            s.raft.last_log_index,
            s.raft.last_applied,
            s.raft.snapshot,
            s.raft.purged,
            s.raft.membership_config.membership(),
        );
    }
}

/// Poll the in-flight membership change, if it completed.
///
/// On success the fuzz driver adopts the new voter set and drops the plan; on
/// failure the same plan is kept for a retry after `cooldown` ticks, so the
/// same target is re-attempted instead of re-rolling a fresh change.
#[allow(clippy::too_many_arguments)]
fn poll_membership(
    inflight: &mut Option<MembershipInflight>,
    pending_plan: &mut Option<MembershipPlan>,
    retry_at: &mut u64,
    stats: &mut MembershipStats,
    active_voters: &mut BTreeSet<NodeId>,
    steps: u64,
    cooldown: u64,
) {
    let done = inflight.as_ref().is_some_and(|i| i.task.done.load(Ordering::SeqCst));
    if !done {
        return;
    }
    let completed = inflight.take().expect("membership inflight must exist");
    if completed.task.success.load(Ordering::SeqCst) {
        *active_voters = completed.new_voters;
        *pending_plan = None;
        stats.applied += 1;
        println!("MEMBERSHIP: Applied change, voters={active_voters:?}");
    } else {
        stats.failed += 1;
        *retry_at = steps + cooldown;
        println!("MEMBERSHIP: Change failed; retrying same plan at step {retry_at} (voters={active_voters:?})");
    }
}

/// Step the sim until [`liveness::check_converged`] passes or `deadline_in`
/// ticks elapse. On timeout, records a violation (tagged with `label`) and
/// dumps per-node state.
fn wait_converged(
    sim: &mut turmoil::Sim,
    st: &mut IterationState,
    cluster_state: &Arc<Mutex<ClusterState>>,
    workload: &mut WorkloadCtx,
    running: &Arc<AtomicBool>,
    deadline_in: u64,
    label: &str,
) -> Option<liveness::Converged> {
    let deadline = st.steps + deadline_in;
    let mut last_reason = String::from("convergence never evaluated");
    let mut finalize_inflight: Option<Arc<AtomicBool>> = None;
    while running.load(Ordering::Relaxed) && st.steps < deadline {
        workload.reads.maybe_enqueue(st.steps, &workload.read_queue);
        if !st.step_and_check(sim, cluster_state, &workload.history) {
            return None;
        }
        if st.steps.is_multiple_of(CONVERGE_CHECK_EVERY) {
            let snapshots = cluster_state.lock().unwrap().get_all_full_snapshots();
            match liveness::check_converged(&snapshots) {
                Ok(c) => {
                    println!(
                        "LIVENESS: {label} converged at step {} (leader n{}, members {:?})",
                        st.steps, c.leader, c.members
                    );
                    return Some(c);
                }
                Err(reason) => last_reason = reason,
            }
            // A joint config blocks convergence and openraft never finalizes
            // one on its own; act as the operator whenever one is visible.
            maybe_finalize_joint(sim, st.steps, cluster_state, &mut finalize_inflight);
        }
    }
    if running.load(Ordering::Relaxed) {
        st.record_violation(format!(
            "Step {}: Liveness: {label} failed to converge within {deadline_in} ticks: {last_reason}",
            st.steps
        ));
        dump_cluster_state(cluster_state);
    }
    None
}

/// Liveness phase: remove all faults, then require the cluster to completely
/// heal (phase A), serve fresh writes and linearizable reads (phase B), and
/// re-converge so the durability scan runs on settled state (phase C).
///
/// Any unmet requirement is recorded as a violation in `st`.
fn run_liveness_phase(
    sim: &mut turmoil::Sim,
    st: &mut IterationState,
    cluster_state: &Arc<Mutex<ClusterState>>,
    derived: &DerivedConfig,
    workload: &mut WorkloadCtx,
    pending_bounces: &mut Vec<(NodeId, u64)>,
    running: &Arc<AtomicBool>,
) {
    println!("\n=== LIVENESS PHASE at step {}: removing all faults ===", st.steps);
    for (id, _) in pending_bounces.drain(..) {
        bounce_node(sim, id);
    }
    heal_network(sim, derived.max_potential_nodes, derived);
    // Quiescence: scheduled writes stop, so a cluster that needs new proposals
    // to unstick replication cannot heal by accident. Reads keep flowing; they
    // propose nothing.
    workload.paused.store(true, Ordering::SeqCst);

    // Phase A: full convergence.
    let Some(_) = wait_converged(
        sim,
        st,
        cluster_state,
        workload,
        running,
        LIVENESS_CONVERGE_DEADLINE,
        "healed cluster",
    ) else {
        return;
    };

    // Phase B: the healed cluster must serve fresh writes and reads.
    let base = workload.history.lock().unwrap().stats.clone();
    let deadline = st.steps + SERVICE_CHECK_DEADLINE;
    let mut next_op = st.steps;
    loop {
        if !running.load(Ordering::Relaxed) {
            return;
        }
        let stats = workload.history.lock().unwrap().stats.clone();
        let writes_done = stats.writes_acked - base.writes_acked >= SERVICE_OPS;
        let reads_done = stats.reads_ok - base.reads_ok >= SERVICE_OPS;
        if writes_done && reads_done {
            println!(
                "LIVENESS: service check passed at step {} ({SERVICE_OPS} writes acked, {SERVICE_OPS} reads ok)",
                st.steps
            );
            break;
        }
        if st.steps >= deadline {
            st.record_violation(format!(
                "Step {}: Liveness: healed cluster failed to serve within {SERVICE_CHECK_DEADLINE} ticks: \
                 {}/{SERVICE_OPS} writes acked, {}/{SERVICE_OPS} reads ok",
                st.steps,
                stats.writes_acked - base.writes_acked,
                stats.reads_ok - base.reads_ok,
            ));
            dump_cluster_state(cluster_state);
            return;
        }
        if st.steps >= next_op {
            next_op = st.steps + 250;
            if !writes_done && let Some(raft) = cluster_state.lock().unwrap().find_leader() {
                let req = workload.schedule.build_request();
                enqueue_write(&workload.driver_queue, &workload.history, raft, req);
            }
            if !reads_done {
                let key = workload.reads.pick_key();
                workload.read_queue.lock().unwrap().push_back(key);
            }
        }
        if !st.step_and_check(sim, cluster_state, &workload.history) {
            return;
        }
    }

    // Phase C: re-converge after the service writes so the durability scan
    // sees settled state. Scanning right after phase B would flag writes that
    // were acked on the leader but not yet applied on followers — follower
    // apply lag is normal, not data loss.
    let Some(converged) = wait_converged(
        sim,
        st,
        cluster_state,
        workload,
        running,
        LIVENESS_CONVERGE_DEADLINE,
        "post-service cluster",
    ) else {
        return;
    };

    // Durability scan: every member's settled state machine must hold every
    // acked write (or a later one) for each key.
    let member_sms: Vec<(NodeId, StateMachineData)> = cluster_state
        .lock()
        .unwrap()
        .get_all_full_snapshots()
        .into_iter()
        .filter(|(id, _)| converged.members.contains(id))
        .map(|(id, s)| (id, s.sm))
        .collect();
    workload.history.lock().unwrap().check_final_durability(&member_sms);
    let leaked: Vec<_> = workload.history.lock().unwrap().drain_violations();
    if leaked.is_empty() {
        println!("LIVENESS: durability check passed on members {:?}", converged.members);
    } else {
        for v in leaked {
            st.record_violation(format!("Step {}: {v}", st.steps));
        }
    }
}

fn run_single_iteration(
    iteration_seed: u64,
    derived: &DerivedConfig,
    max_steps: u64,
    running: Arc<AtomicBool>,
) -> FuzzResult {
    // Reset the `futures_util::select!` shuffle RNG (a process-wide
    // thread-local, see the vendored futures-util patch). Without this, RNG
    // state leaks across iterations and an in-process iteration is not
    // reproducible by a fresh-process `--reproduce` run of the same seed.
    futures_util::reseed(iteration_seed);

    let rng = Box::new(SmallRng::seed_from_u64(iteration_seed));
    // Sized so a healthy run can never hit the simulation-duration limit:
    // safety phase + both liveness deadlines + slack. Hitting it anyway is a
    // harness failure, reported through `step_tick`.
    let sim_duration_ms = max_steps + LIVENESS_CONVERGE_DEADLINE + SERVICE_CHECK_DEADLINE + 100_000;
    let mut sim = turmoil::Builder::new()
        .simulation_duration(Duration::from_millis(sim_duration_ms))
        .min_message_latency(Duration::from_millis(derived.min_message_latency_ms))
        .max_message_latency(Duration::from_millis(derived.max_message_latency_ms))
        .fail_rate(derived.fail_rate)
        .enable_random_order()
        .tcp_capacity(65536)
        .build_with_rng(rng);

    let raft_config = Arc::new(openraft::Config {
        heartbeat_interval: derived.heartbeat_interval,
        election_timeout_min: derived.election_timeout_min,
        election_timeout_max: derived.election_timeout_max,
        snapshot_policy: openraft::SnapshotPolicy::LogsSinceLast(derived.snapshot_logs_threshold),
        max_in_snapshot_log_to_keep: derived.max_in_snapshot_log_to_keep,
        replication_lag_threshold: derived.replication_lag_threshold,
        enable_leader_restore: Some(derived.enable_leader_restore),
        enable_pre_vote: Some(derived.enable_pre_vote),
        ..Default::default()
    });

    let cluster_state = Arc::new(Mutex::new(ClusterState::new()));
    let workload_paused = Arc::new(AtomicBool::new(false));
    let workload_attempts = Arc::new(AtomicU64::new(0));
    let history = Arc::new(Mutex::new(ClientHistory::default()));
    let driver_queue: DriverQueue = Arc::new(Mutex::new(VecDeque::new()));
    let read_queue: ReadQueue = Arc::new(Mutex::new(VecDeque::new()));

    // Potential cluster members: every host we spawn has an entry here so the
    // membership client can construct a `Node` when adding a new learner.
    let potential_nodes: BTreeMap<NodeId, Node> = (1..=derived.max_potential_nodes)
        .map(|id| {
            (id, Node {
                addr: format!("{}:9000", host_name(id)),
            })
        })
        .collect();

    // Initial cluster members: only these are passed to `raft.initialize()` so
    // the bootstrap membership matches `num_initial_nodes`. Non-initial hosts
    // still come up, run uninitialized, and join later via add_learner +
    // change_membership.
    let initial_nodes: BTreeMap<NodeId, Node> = potential_nodes
        .iter()
        .take(derived.num_initial_nodes)
        .map(|(id, node)| (*id, node.clone()))
        .collect();

    for id in 1..=derived.max_potential_nodes {
        register_node_storage(id, &cluster_state);
        spawn_host(
            &mut sim,
            id,
            raft_config.clone(),
            cluster_state.clone(),
            iteration_seed,
            initial_nodes.clone(),
        );
    }

    let mut network_chaos = derived.enable_chaos.then(|| NetworkChaos::new(iteration_seed.wrapping_add(1000)));

    sim.client("workload-driver", driver_loop(driver_queue.clone(), history.clone()));
    sim.client(
        "read-client",
        read_client_loop(read_queue.clone(), cluster_state.clone(), history.clone()),
    );

    // Main simulation loop
    let mut st = IterationState::new();
    let mut chaos_rng = StdRng::seed_from_u64(iteration_seed.wrapping_add(3000));
    let mut member_rng = StdRng::seed_from_u64(iteration_seed.wrapping_add(5000));
    let mut workload = WorkloadCtx {
        schedule: WorkloadSchedule::new(
            iteration_seed.wrapping_add(2000),
            derived.key_space,
            derived.quiet_window_chance,
            max_steps.saturating_sub(derived.pre_liveness_quiet_ticks).max(1),
            workload_attempts.clone(),
        ),
        reads: ReadSchedule::new(iteration_seed.wrapping_add(4000), derived.key_space),
        driver_queue,
        read_queue,
        history: history.clone(),
        paused: workload_paused.clone(),
    };
    let mut trigger_schedule = derived
        .enable_trigger_ops
        .then(|| TriggerSchedule::new(iteration_seed.wrapping_add(6000), derived.trigger_interval));
    let mut active_voters: BTreeSet<NodeId> = (1..=derived.num_initial_nodes as u64).collect();
    let mut membership_inflight: Option<MembershipInflight> = None;
    let mut membership_pending: Option<MembershipPlan> = None;
    let mut membership_retry_at: u64 = 0;
    let mut membership_stats = MembershipStats::default();
    let membership_cooldown = (derived.membership_interval / 4).max(2000);

    // Pending bounces: (node_id, step_at_which_to_bounce). When a crash is
    // triggered, we push an entry here and later bounce the node when the
    // simulation reaches the scheduled step.
    let mut pending_bounces: Vec<(NodeId, u64)> = Vec::new();

    println!("Starting simulation (safety phase)...");

    while running.load(Ordering::Relaxed) && st.steps < max_steps {
        poll_membership(
            &mut membership_inflight,
            &mut membership_pending,
            &mut membership_retry_at,
            &mut membership_stats,
            &mut active_voters,
            st.steps,
            membership_cooldown,
        );

        if let Some(network_chaos) = &mut network_chaos {
            network_chaos.maybe_apply(st.steps, &mut sim, derived.max_potential_nodes, derived, &cluster_state);
        }

        // Membership changes: a fresh plan on the interval, or a failed plan's
        // retry once its cooldown expires.
        let new_due =
            membership_pending.is_none() && st.steps > 0 && st.steps.is_multiple_of(derived.membership_interval);
        let retry_due = membership_pending.is_some() && st.steps >= membership_retry_at;
        if membership_inflight.is_none() && (new_due || retry_due) {
            let plan = match membership_pending.clone() {
                Some(plan) => {
                    println!(
                        "MEMBERSHIP: Retrying change to {:?} (retain={})",
                        plan.desired, plan.retain
                    );
                    Some(plan)
                }
                None => {
                    let plan = new_membership_plan(&mut member_rng, &active_voters, &potential_nodes);
                    if let Some(p) = &plan {
                        println!("MEMBERSHIP: Requesting change to {:?} (retain={})", p.desired, p.retain);
                    }
                    plan
                }
            };
            if let Some(plan) = plan {
                match schedule_membership_change(
                    &mut sim,
                    st.steps,
                    cluster_state.clone(),
                    &plan,
                    workload_paused.clone(),
                ) {
                    Some(task) => {
                        membership_stats.attempts += 1;
                        membership_inflight = Some(MembershipInflight {
                            task,
                            new_voters: plan.desired.clone(),
                        });
                        membership_pending = Some(plan);
                    }
                    None => {
                        // No leader right now; retry the same plan later.
                        membership_pending = Some(plan);
                        membership_retry_at = st.steps + membership_cooldown;
                    }
                }
            }
        }

        // Bounce any nodes whose crash window has expired.
        pending_bounces.retain(|(id, bounce_at)| {
            if st.steps >= *bounce_at {
                bounce_node(&mut sim, *id);
                false
            } else {
                true
            }
        });

        // Crash a random voter and schedule its bounce after a downtime
        // window. Two flavors:
        //
        // - Short outage (majority case): window straddles `election_timeout_max`, mixing "no re-election"
        //   with "leader churn / quorum loss" on restart.
        // - Long outage (rare, `long_outage_chance`): 5k-15k ticks, so the crashed node falls behind by
        //   more than `replication_lag_threshold` and must receive a snapshot install instead of log
        //   shipping when it rejoins.
        if st.steps > 0 && st.steps.is_multiple_of(derived.chaos_interval) && chaos_rng.gen_bool(derived.restart_chance)
        {
            let crashable: Vec<_> =
                active_voters.iter().copied().filter(|id| !pending_bounces.iter().any(|(p, _)| p == id)).collect();
            if !crashable.is_empty() {
                let victim = crashable[chaos_rng.gen_range(0..crashable.len())];
                let is_long_outage = chaos_rng.gen_bool(derived.long_outage_chance);
                let downtime = if is_long_outage {
                    chaos_rng.gen_range(derived.long_outage_min_ticks..=derived.long_outage_max_ticks)
                } else {
                    let min_downtime = derived.election_timeout_max / 2;
                    let max_downtime = derived.election_timeout_max * 2;
                    chaos_rng.gen_range(min_downtime..=max_downtime)
                };
                crash_node(&mut sim, victim, &cluster_state);
                pending_bounces.push((victim, st.steps + downtime));
                let kind = if is_long_outage { "CRASH(long)" } else { "CRASH" };
                println!(
                    "{kind}: node {victim} for {downtime} ticks (bounce at step {})",
                    st.steps + downtime
                );
            }
        }

        workload.schedule.maybe_enqueue(
            st.steps,
            &workload.driver_queue,
            &cluster_state,
            &workload.paused,
            &workload.history,
        );
        workload.reads.maybe_enqueue(st.steps, &workload.read_queue);
        if let Some(triggers) = &mut trigger_schedule {
            triggers.maybe_enqueue(
                st.steps,
                &workload.driver_queue,
                derived.max_potential_nodes,
                &cluster_state,
            );
        }

        if !st.step_and_check(&mut sim, &cluster_state, &history) {
            return st.into_result();
        }

        // Progress report
        if st.steps.is_multiple_of(5000) {
            let metrics = cluster_state.lock().unwrap().get_all_metrics();
            let leaders: Vec<_> = metrics.iter().filter(|(_, m)| m.state.is_leader()).map(|(id, _)| *id).collect();
            let max_term = metrics.iter().map(|(_, m)| m.vote.leader_id().term).max().unwrap_or(0);
            let max_committed = metrics.iter().filter_map(|(_, m)| m.local_committed).max_by_key(|id| id.index());
            let stats = history.lock().unwrap().stats.clone();
            println!(
                "[Step {}] leaders={leaders:?}, term={max_term}, voters={active_voters:?}, \
                 checks={}, {stats}, max_committed={max_committed:?}",
                st.steps, st.invariant_checks,
            );
        }
    }

    if !running.load(Ordering::Relaxed) {
        println!("Interrupted at step {}", st.steps);
    } else {
        println!("Safety phase complete at step {} (max_steps: {max_steps})", st.steps);
        run_liveness_phase(
            &mut sim,
            &mut st,
            &cluster_state,
            derived,
            &mut workload,
            &mut pending_bounces,
            &running,
        );
    }

    let metrics = cluster_state.lock().unwrap().get_all_metrics();
    let leaders: Vec<_> = metrics.iter().filter(|(_, m)| m.state.is_leader()).map(|(id, _)| *id).collect();
    let max_term = metrics.iter().map(|(_, m)| m.vote.leader_id().term).max().unwrap_or(0);
    let max_committed = metrics.iter().filter_map(|(_, m)| m.local_committed).max_by_key(|id| id.index());
    let stats = history.lock().unwrap().stats.clone();
    println!(
        "Final summary: leaders={leaders:?}, term={max_term}, voters={active_voters:?}, \
         {stats}, {membership_stats}, max_committed={max_committed:?}",
    );

    st.into_result()
}
