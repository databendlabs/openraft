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
use tests_turmoil::typ::*;

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
            max_in_snapshot_log_to_keep: rng.gen_range(30..=80),
            replication_lag_threshold: snapshot_logs_threshold * 2,
            long_outage_chance: rng.gen_range(0.1..=0.3), // 10-30% of crashes
            long_outage_min_ticks: 5000,
            long_outage_max_ticks: 15000,
            enable_leader_restore,
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
        write!(f, "  enable_leader_restore: {}", self.enable_leader_restore)
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

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("openraft=trace".parse().unwrap())
                .add_directive("tests_turmoil=debug".parse().unwrap())
                .add_directive("info".parse().unwrap()),
        )
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

        let result = run_single_iteration(iteration_seed, &derived, max_steps, running.clone());
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
        sim: &turmoil::Sim,
        max_nodes: u64,
        cluster_state: &Arc<Mutex<ClusterState>>,
    ) {
        if steps < self.next_step {
            return;
        }

        apply_network_chaos(sim, &mut self.rng, max_nodes, cluster_state);
        self.next_step = steps + self.rng.gen_range(1000..5000);
    }
}

struct WorkloadSchedule {
    rng: StdRng,
    next_step: u64,
    next_serial: u64,
    attempts: Arc<AtomicU64>,
}

type WorkloadQueue = Arc<Mutex<VecDeque<(Arc<Raft>, Request)>>>;

struct MembershipTask {
    done: Arc<AtomicBool>,
    success: Arc<AtomicBool>,
}

struct MembershipInflight {
    task: MembershipTask,
    new_voters: BTreeSet<NodeId>,
    added_node: Option<NodeId>,
}

impl WorkloadSchedule {
    fn new(seed: u64, attempts: Arc<AtomicU64>) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let next_step = rng.gen_range(10..50);
        Self {
            rng,
            next_step,
            next_serial: 0,
            attempts,
        }
    }

    fn maybe_enqueue(
        &mut self,
        steps: u64,
        queue: &WorkloadQueue,
        cluster_state: &Arc<Mutex<ClusterState>>,
        paused: &Arc<AtomicBool>,
    ) {
        if paused.load(Ordering::SeqCst) || steps < self.next_step {
            return;
        }

        if let Some(raft) = cluster_state.lock().unwrap().find_leader() {
            let serial = self.next_serial;
            let req = Request {
                client_id: "workload".to_string(),
                serial,
                key: format!("key-{}", serial % 1000),
                value: format!("value-{}-{}", serial, self.rng.r#gen::<u32>()),
            };

            queue.lock().unwrap().push_back((raft, req));
            self.next_serial += 1;
            self.attempts.store(self.next_serial, Ordering::SeqCst);
        }

        self.next_step = steps + self.rng.gen_range(10..50);
    }
}

fn apply_network_chaos(sim: &turmoil::Sim, rng: &mut StdRng, max_nodes: u64, cluster_state: &Arc<Mutex<ClusterState>>) {
    match rng.gen_range(0..6) {
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
        _ => unreachable!(),
    }
}

async fn membership_change_once(
    raft: Arc<Raft>,
    learners_to_add: Vec<(NodeId, Node)>,
    new_set: BTreeSet<NodeId>,
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

    println!("MEMBERSHIP-CLIENT: executing change to {new_set:?}");
    match tokio::time::timeout(
        Duration::from_millis(5000),
        raft.change_membership(new_set.clone(), false),
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

fn schedule_membership_change(
    sim: &mut turmoil::Sim,
    steps: u64,
    cluster_state: Arc<Mutex<ClusterState>>,
    learners_to_add: Vec<(NodeId, Node)>,
    new_set: BTreeSet<NodeId>,
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
            learners_to_add,
            new_set,
            workload_paused,
            done.clone(),
            success.clone(),
        ),
    );
    Some(MembershipTask { done, success })
}

async fn workload_driver_loop(queue: WorkloadQueue) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let requests: Vec<_> = queue.lock().unwrap().drain(..).collect();
        for (raft, req) in requests {
            tokio::spawn(async move {
                let _ = raft.client_write(req).await;
            });
        }

        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// Advance the simulation by one tick.
///
/// `Sim::step()` returns `Ok(true)` when all client hosts have exited. The
/// workload client runs forever, so that case should be impossible; if it ever
/// happens, the fuzz harness has lost its driver and must abort rather than
/// keep spinning.
fn step_tick(sim: &mut turmoil::Sim) -> Result<(), String> {
    match sim.step() {
        Ok(false) => Ok(()),
        Ok(true) => Err("All fuzz clients exited unexpectedly".to_string()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("duration") || msg.contains("without completing") {
                Err("Simulation duration reached".to_string())
            } else {
                Err(format!("Simulation error: {e}"))
            }
        }
    }
}

fn run_single_iteration(
    iteration_seed: u64,
    derived: &DerivedConfig,
    max_steps: u64,
    running: Arc<AtomicBool>,
) -> FuzzResult {
    let rng = Box::new(SmallRng::seed_from_u64(iteration_seed));
    let mut sim = turmoil::Builder::new()
        .simulation_duration(Duration::from_secs(3600))
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
        ..Default::default()
    });

    let cluster_state = Arc::new(Mutex::new(ClusterState::new()));
    let workload_paused = Arc::new(AtomicBool::new(false));
    let workload_attempts = Arc::new(AtomicU64::new(0));
    let workload_queue: WorkloadQueue = Arc::new(Mutex::new(VecDeque::new()));

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

    sim.client("workload-driver", workload_driver_loop(workload_queue.clone()));

    // Main simulation loop
    let mut steps: u64 = 0;
    let mut invariant_checks: u64 = 0;
    let mut violations: Vec<String> = Vec::new();
    let mut chaos_rng = StdRng::seed_from_u64(iteration_seed.wrapping_add(3000));
    let mut member_rng = StdRng::seed_from_u64(iteration_seed.wrapping_add(5000));
    let mut workload_schedule = WorkloadSchedule::new(iteration_seed.wrapping_add(2000), workload_attempts.clone());
    let mut active_voters: BTreeSet<NodeId> = (1..=derived.num_initial_nodes as u64).collect();
    let mut next_node_id = (derived.num_initial_nodes as u64) + 1;
    let mut invariants = InvariantChecker::default();
    let mut membership_inflight: Option<MembershipInflight> = None;

    // Pending bounces: (node_id, step_at_which_to_bounce). When a crash is
    // triggered, we push an entry here and later bounce the node when the
    // simulation reaches the scheduled step.
    let mut pending_bounces: Vec<(NodeId, u64)> = Vec::new();

    println!("Starting simulation...");

    while running.load(Ordering::Relaxed) && steps < max_steps {
        if membership_inflight.as_ref().is_some_and(|inflight| inflight.task.done.load(Ordering::SeqCst)) {
            let inflight = membership_inflight.take().expect("membership inflight must exist");
            if inflight.task.success.load(Ordering::SeqCst) {
                active_voters = inflight.new_voters;
                if let Some(added_node) = inflight.added_node {
                    next_node_id = next_node_id.max(added_node + 1);
                }
                println!("MEMBERSHIP: Applied change, voters={active_voters:?}");
            } else {
                println!("MEMBERSHIP: Change failed; keeping voters={active_voters:?}");
            }
        }

        if let Some(network_chaos) = &mut network_chaos {
            network_chaos.maybe_apply(steps, &sim, derived.max_potential_nodes, &cluster_state);
        }

        // Membership changes
        if steps > 0 && steps.is_multiple_of(derived.membership_interval) {
            if membership_inflight.is_some() {
                println!("MEMBERSHIP: Previous change still in flight at step {steps}; skipping");
            } else {
                let add = active_voters.len() < 3 || (active_voters.len() < 7 && member_rng.gen_bool(0.7));
                if add {
                    if next_node_id <= derived.max_potential_nodes {
                        println!("MEMBERSHIP: Requesting add node {next_node_id}...");
                        let mut next_voters = active_voters.clone();
                        next_voters.insert(next_node_id);
                        if let Some(done) = schedule_membership_change(
                            &mut sim,
                            steps,
                            cluster_state.clone(),
                            potential_nodes
                                .get(&next_node_id)
                                .cloned()
                                .map(|node| vec![(next_node_id, node)])
                                .unwrap_or_default(),
                            next_voters.clone(),
                            workload_paused.clone(),
                        ) {
                            membership_inflight = Some(MembershipInflight {
                                task: done,
                                new_voters: next_voters,
                                added_node: Some(next_node_id),
                            });
                        }
                    }
                } else if active_voters.len() > 3 {
                    let voters: Vec<NodeId> = active_voters.iter().copied().collect();
                    let victim = voters[member_rng.gen_range(0..voters.len())];
                    println!("MEMBERSHIP: Requesting remove node {victim}...");
                    let mut next_voters = active_voters.clone();
                    next_voters.remove(&victim);
                    if let Some(done) = schedule_membership_change(
                        &mut sim,
                        steps,
                        cluster_state.clone(),
                        Vec::new(),
                        next_voters.clone(),
                        workload_paused.clone(),
                    ) {
                        membership_inflight = Some(MembershipInflight {
                            task: done,
                            new_voters: next_voters,
                            added_node: None,
                        });
                    }
                }
            }
        }

        // Bounce any nodes whose crash window has expired.
        pending_bounces.retain(|(id, bounce_at)| {
            if steps >= *bounce_at {
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
        if steps > 0 && steps.is_multiple_of(derived.chaos_interval) && chaos_rng.gen_bool(derived.restart_chance) {
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
                pending_bounces.push((victim, steps + downtime));
                let kind = if is_long_outage { "CRASH(long)" } else { "CRASH" };
                println!(
                    "{kind}: node {victim} for {downtime} ticks (bounce at step {})",
                    steps + downtime
                );
            }
        }

        workload_schedule.maybe_enqueue(steps, &workload_queue, &cluster_state, &workload_paused);

        if let Err(msg) = step_tick(&mut sim) {
            println!("{msg} at step {steps}");
            return FuzzResult {
                steps_completed: steps,
                invariant_checks,
                violations,
            };
        }
        steps += 1;

        // Check invariants
        let (snapshots, durable_logs) = {
            let state = cluster_state.lock().unwrap();
            (state.get_all_full_snapshots(), state.get_all_durable_log_ids())
        };
        invariant_checks += 1;
        let result = invariants.check_with_durable_logs(&snapshots, &durable_logs);
        if !result.violations.is_empty() {
            for v in &result.violations {
                let msg = format!("Step {steps}: {v:?}");
                println!("VIOLATION: {msg}");
                violations.push(msg);
            }
            return FuzzResult {
                steps_completed: steps,
                invariant_checks,
                violations,
            };
        }

        // Progress report
        if steps.is_multiple_of(5000) {
            let metrics = cluster_state.lock().unwrap().get_all_metrics();
            let leaders: Vec<_> = metrics.iter().filter(|(_, m)| m.state.is_leader()).map(|(id, _)| *id).collect();
            let max_term = metrics.iter().map(|(_, m)| m.vote.leader_id().term).max().unwrap_or(0);
            let max_committed = metrics.iter().filter_map(|(_, m)| m.committed).max_by_key(|id| id.index());
            println!(
                "[Step {steps}] leaders={leaders:?}, term={max_term}, \
                 voters={active_voters:?}, checks={invariant_checks}, \
                 workload_attempts={}, max_committed={max_committed:?}",
                workload_attempts.load(Ordering::SeqCst),
            );
        }
    }

    if !running.load(Ordering::Relaxed) {
        println!("Interrupted at step {steps}");
    } else {
        println!("Reached max steps: {max_steps}");
    }

    let metrics = cluster_state.lock().unwrap().get_all_metrics();
    let leaders: Vec<_> = metrics.iter().filter(|(_, m)| m.state.is_leader()).map(|(id, _)| *id).collect();
    let max_term = metrics.iter().map(|(_, m)| m.vote.leader_id().term).max().unwrap_or(0);
    let max_committed = metrics.iter().filter_map(|(_, m)| m.committed).max_by_key(|id| id.index());
    println!(
        "Final summary: leaders={leaders:?}, term={max_term}, voters={active_voters:?}, \
         workload_attempts={}, max_committed={max_committed:?}",
        workload_attempts.load(Ordering::SeqCst),
    );

    FuzzResult {
        steps_completed: steps,
        invariant_checks,
        violations,
    }
}
