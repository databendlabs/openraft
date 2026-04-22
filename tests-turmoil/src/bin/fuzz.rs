//! Tick-based fuzzer for OpenRaft turmoil tests.
//!
//! Modes:
//!   Fuzz mode:      fuzz --seed <SEED> --max-steps <N> [--crash-file <PATH>]
//!   Reproduce mode: fuzz --reproduce <ITERATION_SEED> --max-steps <N> [--crash-file <PATH>]

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use clap::Parser;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use rand::rngs::StdRng;
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
}

impl DerivedConfig {
    fn from_seed(seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);
        let heartbeat_interval = 50 + rng.gen_range(0..100);
        let election_timeout_min = heartbeat_interval * rng.gen_range(2..4);
        let snapshot_logs_threshold = rng.gen_range(100..=250);
        Self {
            num_initial_nodes: 3 + rng.gen_range(0..3), // 3-5
            max_potential_nodes: 10,
            fail_rate: rng.gen_range(0.0..0.08), // 0-8%
            heartbeat_interval,
            election_timeout_min,
            election_timeout_max: election_timeout_min + rng.gen_range(100..500),
            enable_chaos: rng.gen_bool(0.8),           // 80% chance
            restart_chance: rng.gen_range(0.01..0.05), // 1-5%
            chaos_interval: rng.gen_range(2000..5000),
            membership_interval: rng.gen_range(10000..25000),
            snapshot_logs_threshold,
            max_in_snapshot_log_to_keep: rng.gen_range(30..=80),
            replication_lag_threshold: snapshot_logs_threshold * 2,
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
        write!(f, "  replication_lag_threshold: {}", self.replication_lag_threshold)
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
            "--- Iteration {} (seed: {}, nodes: {}, fail_rate: {:.2}%, chaos: {}) ---",
            iteration + 1,
            iteration_seed,
            derived.num_initial_nodes,
            derived.fail_rate * 100.0,
            derived.enable_chaos
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

async fn chaos_agent_loop(seed: u64, max_nodes: u64) -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = StdRng::seed_from_u64(seed);
    loop {
        tokio::time::sleep(Duration::from_millis(rng.gen_range(1000..5000))).await;
        match rng.gen_range(0..5) {
            0 => {
                let victim = rng.gen_range(1..=max_nodes);
                for i in 1..=max_nodes {
                    if i != victim {
                        turmoil::partition(host_name(victim), host_name(i));
                    }
                }
            }
            1 => {
                for i in 1..=max_nodes {
                    for j in (i + 1)..=max_nodes {
                        turmoil::repair(host_name(i), host_name(j));
                    }
                }
            }
            2 => {
                let a = rng.gen_range(1..=max_nodes);
                let mut b = rng.gen_range(1..=max_nodes);
                while b == a {
                    b = rng.gen_range(1..=max_nodes);
                }
                turmoil::hold(host_name(a), host_name(b));
            }
            3 => {
                for i in 1..=max_nodes {
                    for j in 1..=max_nodes {
                        if i != j {
                            turmoil::release(host_name(i), host_name(j));
                        }
                    }
                }
            }
            _ => {} // no-op: let system stabilize
        }
    }
}

async fn membership_agent_loop(
    cluster_state: Arc<Mutex<ClusterState>>,
    next_membership: Arc<Mutex<Option<HashSet<NodeId>>>>,
    potential_nodes: BTreeMap<NodeId, Node>,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;

        let Some(new_set) = next_membership.lock().unwrap().take() else {
            continue;
        };

        let leader = cluster_state.lock().unwrap().find_leader();
        let Some(raft) = leader else {
            // No leader found; put the membership change back for retry.
            next_membership.lock().unwrap().get_or_insert(new_set);
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        };

        // openraft requires a node to be a learner before promotion to voter.
        // Add each target as learner; ignore "already known" errors — we do
        // not track membership here and prefer idempotent retries.
        for id in &new_set {
            let Some(node) = potential_nodes.get(id).cloned() else {
                continue;
            };
            let _ = raft.add_learner(*id, node, true).await;
        }

        println!("MEMBERSHIP-AGENT: executing change to {new_set:?}");
        let _ = raft.change_membership(new_set, false).await;
    }
}

async fn workload_loop(cluster_state: Arc<Mutex<ClusterState>>, seed: u64) -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut write_count = 0u64;
    loop {
        tokio::time::sleep(Duration::from_millis(rng.gen_range(10..50))).await;
        let leader = cluster_state.lock().unwrap().find_leader();
        if let Some(raft) = leader {
            let req = Request {
                client_id: "workload".to_string(),
                serial: write_count,
                key: format!("key-{}", write_count % 1000),
                value: format!("value-{}-{}", write_count, rng.r#gen::<u32>()),
            };
            if raft.client_write(req).await.is_ok() {
                write_count += 1;
            }
        }
    }
}

/// Advance the simulation by one tick.
///
/// `Sim::step()` returns `Ok(true)` when all client hosts have exited. Our
/// fuzz clients (`workload`, `membership-agent`, `chaos-agent`) run forever,
/// so that case should be impossible; if it ever happens, the fuzz harness
/// has lost its drivers and we must abort rather than keep spinning.
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
        ..Default::default()
    });

    let cluster_state = Arc::new(Mutex::new(ClusterState::new()));
    let next_membership = Arc::new(Mutex::new(None::<HashSet<NodeId>>));

    // Potential cluster members: every host we spawn has an entry here so the
    // membership agent can construct a `Node` when adding a new learner.
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

    if derived.enable_chaos {
        sim.client(
            "chaos-agent",
            chaos_agent_loop(iteration_seed.wrapping_add(1000), derived.max_potential_nodes),
        );
    }
    sim.client(
        "membership-agent",
        membership_agent_loop(cluster_state.clone(), next_membership.clone(), potential_nodes.clone()),
    );
    sim.client(
        "workload",
        workload_loop(cluster_state.clone(), iteration_seed.wrapping_add(2000)),
    );

    // Main simulation loop
    let mut steps: u64 = 0;
    let mut invariant_checks: u64 = 0;
    let mut violations: Vec<String> = Vec::new();
    let mut chaos_rng = StdRng::seed_from_u64(iteration_seed.wrapping_add(3000));
    let mut member_rng = StdRng::seed_from_u64(iteration_seed.wrapping_add(5000));
    let mut active_voters: HashSet<NodeId> = (1..=derived.num_initial_nodes as u64).collect();
    let mut next_node_id = (derived.num_initial_nodes as u64) + 1;
    let mut invariants = InvariantChecker::default();

    // Pending bounces: (node_id, step_at_which_to_bounce). When a crash is
    // triggered, we push an entry here and later bounce the node when the
    // simulation reaches the scheduled step.
    let mut pending_bounces: Vec<(NodeId, u64)> = Vec::new();

    println!("Starting simulation...");

    while running.load(Ordering::Relaxed) && steps < max_steps {
        // Membership changes
        if steps > 0 && steps.is_multiple_of(derived.membership_interval) {
            let add = active_voters.len() < 3 || (active_voters.len() < 7 && member_rng.gen_bool(0.7));
            if add {
                if next_node_id <= derived.max_potential_nodes {
                    println!("MEMBERSHIP: Requesting add node {next_node_id}...");
                    active_voters.insert(next_node_id);
                    next_node_id += 1;
                    *next_membership.lock().unwrap() = Some(active_voters.clone());
                }
            } else if active_voters.len() > 3 {
                let victim = *active_voters.iter().next().unwrap();
                println!("MEMBERSHIP: Requesting remove node {victim}...");
                active_voters.remove(&victim);
                *next_membership.lock().unwrap() = Some(active_voters.clone());
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

        // Crash a random voter for a bounded downtime, then schedule its
        // bounce. We pick the window to straddle `election_timeout_max` so
        // the fuzz mixes "short outage, no re-election" with "long outage,
        // leader churn / quorum loss".
        if steps > 0 && steps.is_multiple_of(derived.chaos_interval) && chaos_rng.gen_bool(derived.restart_chance) {
            let crashable: Vec<_> =
                active_voters.iter().copied().filter(|id| !pending_bounces.iter().any(|(p, _)| p == id)).collect();
            if !crashable.is_empty() {
                let victim = crashable[chaos_rng.gen_range(0..crashable.len())];
                let min_downtime = derived.election_timeout_max / 2;
                let max_downtime = derived.election_timeout_max * 2;
                let downtime = chaos_rng.gen_range(min_downtime..=max_downtime);
                crash_node(&mut sim, victim, &cluster_state);
                pending_bounces.push((victim, steps + downtime));
                println!(
                    "CRASH: node {victim} for {downtime} ticks (bounce at step {})",
                    steps + downtime
                );
            }
        }

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
        let snapshots = cluster_state.lock().unwrap().get_all_full_snapshots();
        invariant_checks += 1;
        let result = invariants.check(&snapshots);
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
            println!(
                "[Step {steps}] leaders={leaders:?}, term={max_term}, \
                 voters={active_voters:?}, checks={invariant_checks}"
            );
        }
    }

    if !running.load(Ordering::Relaxed) {
        println!("Interrupted at step {steps}");
    } else {
        println!("Reached max steps: {max_steps}");
    }

    FuzzResult {
        steps_completed: steps,
        invariant_checks,
        violations,
    }
}
