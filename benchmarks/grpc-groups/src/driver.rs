use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::time::Duration;

use anyhow::Context;
use futures::future::try_join_all;
use tokio::time::Instant;
use tokio::time::sleep;

use crate::clock;
use crate::failover_summary::FailoverSummary;
use crate::failover_summary::ready_groups;
use crate::node_process::NodeProcess;
use crate::phase::Phase;
use crate::protobuf as pb;
use crate::raft_config;
use crate::rejoin_summary::RejoinSummary;
use crate::scenario::Scenario;
use crate::window_summary::WindowSummary;

/// How long the `failover` phase waits for the groups of the failed node to recover.
const FAILOVER_LIMIT: Duration = Duration::from_secs(30);

/// How often the `failover` phase asks the surviving nodes how far they got.
const FAILOVER_POLL: Duration = Duration::from_millis(20);

/// How long the `rejoin` phase waits for the returning node to rejoin every group.
const REJOIN_LIMIT: Duration = Duration::from_secs(30);

/// How often the `rejoin` phase asks the returning node how far it got.
const REJOIN_POLL: Duration = Duration::from_millis(20);

/// How long after the arm, which precedes the fault, the faulty node starts to record its rejoin.
/// The node is down or cut off by then, and stays so for at least the `degraded` window, so that it
/// records only what follows its return.
const REJOIN_ARM_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, clap::Args)]
pub(crate) struct RunArgs {
    /// What happens once the groups have settled.
    #[arg(long, value_enum, default_value_t = Scenario::Steady)]
    scenario: Scenario,

    /// The node processes.
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u64).range(1..))]
    nodes: u64,

    /// The groups. Each node holds one member of each.
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(1..))]
    groups: u64,

    /// The worker threads of the tokio runtime of each node.
    #[arg(long, default_value_t = 4)]
    workers: usize,

    /// Seconds to wait once every group has a leader, before the first phase.
    #[arg(long, default_value_t = 10)]
    settle_secs: u64,

    /// Seconds of the `measure` window of `steady`.
    #[arg(long, default_value_t = 30)]
    measure_secs: u64,

    /// Seconds of each window of a fault scenario: `baseline`, `degraded` and `after`.
    #[arg(long, default_value_t = 10)]
    window_secs: u64,

    /// openraft `Config` flags for every node, after `--`, such as `-- --enable-pre-vote true`.
    #[arg(last = true)]
    raft_args: Vec<String>,
}

/// Start the node processes, run the scenario, and print one line per phase.
pub(crate) fn run(args: RunArgs) -> anyhow::Result<()> {
    raft_config::parse(&args.raft_args).context("parse the openraft Config flags")?;
    if args.scenario != Scenario::Steady && args.nodes < 3 {
        anyhow::bail!(
            "{} needs --nodes 3 or more, so that the other nodes keep a quorum",
            args.scenario
        );
    }
    let returns = matches!(args.scenario, Scenario::Hang | Scenario::Partition);
    if returns && Duration::from_secs(args.window_secs) <= REJOIN_ARM_DELAY {
        anyhow::bail!(
            "{} needs --window-secs 1 or more, so that node 1 is still out when it starts to record",
            args.scenario
        );
    }

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(run_scenario(&args))
}

async fn run_scenario(args: &RunArgs) -> anyhow::Result<()> {
    let mut nodes = Vec::new();
    for id in 1..=args.nodes {
        nodes.push(NodeProcess::start(id, args.workers, &args.raft_args).await?);
    }

    set_up_groups(&nodes, args.groups).await?;
    sleep(Duration::from_secs(args.settle_secs)).await;

    let phase = |name| Phase {
        scenario: args.scenario,
        name,
        nodes: args.nodes,
        groups: args.groups,
    };
    match args.scenario {
        Scenario::Steady => {
            print_line(measure_window(&nodes, phase("measure"), args.measure_secs, &[]).await?)?;
        }
        Scenario::Crash => {
            print_line(measure_window(&nodes, phase("baseline"), args.window_secs, &[]).await?)?;

            let node_1 = nodes.remove(0);
            print_line(failover(&nodes, phase("failover"), async move || node_1.crash()).await?)?;

            print_line(measure_window(&nodes, phase("degraded"), args.window_secs, &[1]).await?)?;
        }
        Scenario::Hang => {
            print_line(measure_window(&nodes, phase("baseline"), args.window_secs, &[]).await?)?;

            let node_1 = nodes.remove(0);
            arm_rejoin(&node_1).await?;
            print_line(failover(&nodes, phase("failover"), async || node_1.stop()).await?)?;

            print_line(measure_window(&nodes, phase("degraded"), args.window_secs, &[1]).await?)?;

            print_line(rejoin(&node_1, &nodes, phase("rejoin"), async || node_1.resume()).await?)?;

            nodes.insert(0, node_1);
            print_line(measure_window(&nodes, phase("after"), args.window_secs, &[]).await?)?;
        }
        Scenario::Partition => {
            print_line(measure_window(&nodes, phase("baseline"), args.window_secs, &[]).await?)?;

            // Node 1 keeps running while it is cut off, but `degraded` measures the other nodes
            // alone, as in the other fault scenarios.
            let node_1 = nodes.remove(0);
            arm_rejoin(&node_1).await?;
            print_line(failover(&nodes, phase("failover"), async || node_1.cut().await).await?)?;

            print_line(measure_window(&nodes, phase("degraded"), args.window_secs, &[1]).await?)?;

            print_line(rejoin(&node_1, &nodes, phase("rejoin"), async || node_1.heal().await).await?)?;

            nodes.insert(0, node_1);
            print_line(measure_window(&nodes, phase("after"), args.window_secs, &[]).await?)?;
        }
    }
    Ok(())
}

/// Create the members of every group on `nodes`, initialize each group on its first leader, and
/// wait until every group has committed an entry of its leader's term.
async fn set_up_groups(nodes: &[NodeProcess], groups: u64) -> anyhow::Result<()> {
    let peers = nodes.iter().map(|node| pb::Peer {
        node_id: node.id,
        addr: node.addr.to_string(),
    });
    let peers = peers.collect::<Vec<_>>();

    // Every node creates its members before any node initializes a group, so that no Raft request
    // finds its group missing.
    try_join_all(nodes.iter().map(|node| {
        let mut admin = node.admin();
        let request = pb::SetupRequest {
            peers: peers.clone(),
            groups,
        };
        async move { admin.setup(request).await }
    }))
    .await?;

    try_join_all(nodes.iter().map(|node| {
        let mut admin = node.admin();
        async move { admin.initialize(()).await }
    }))
    .await?;

    try_join_all(nodes.iter().map(|node| {
        let mut admin = node.admin();
        async move { admin.wait_ready(()).await }
    }))
    .await?;
    Ok(())
}

/// Measure a window of `secs` on `nodes`. The leaders leave the followers on the nodes in `down`
/// out of the acknowledgement ages.
async fn measure_window(nodes: &[NodeProcess], phase: Phase, secs: u64, down: &[u64]) -> anyhow::Result<WindowSummary> {
    let begin_terms = try_join_all(nodes.iter().map(|node| {
        let mut admin = node.admin();
        let request = pb::BeginWindowRequest { down: down.to_vec() };
        async move { admin.begin_window(request).await.map(|response| response.into_inner()) }
    }))
    .await?;

    sleep(Duration::from_secs(secs)).await;

    let reports = try_join_all(nodes.iter().map(|node| {
        let mut admin = node.admin();
        async move { admin.end_window(()).await.map(|response| response.into_inner()) }
    }))
    .await?;

    Ok(WindowSummary::new(phase, &begin_terms, &reports))
}

/// Inject `fault` into node 1, and measure how fast `survivors` take over the groups it led.
async fn failover<F>(survivors: &[NodeProcess], phase: Phase, fault: F) -> anyhow::Result<FailoverSummary>
where F: AsyncFnOnce() -> anyhow::Result<()> {
    // Node 1 initialized the groups whose id is a multiple of the node count, and leads them.
    let affected = (0..phase.groups).filter(|group_id| group_id % phase.nodes == 0).collect::<Vec<_>>();

    try_join_all(survivors.iter().map(|node| {
        let mut admin = node.admin();
        let request = pb::ArmFailoverRequest {
            group_ids: affected.clone(),
        };
        async move { admin.arm_failover(request).await }
    }))
    .await?;

    let faulted_ns = clock::monotonic_ns();
    fault().await?;

    let deadline = Instant::now() + FAILOVER_LIMIT;
    let takeovers = loop {
        let takeovers = try_join_all(survivors.iter().map(|node| {
            let mut admin = node.admin();
            async move { admin.get_takeovers(()).await.map(|response| response.into_inner()) }
        }))
        .await?;

        let recovered = ready_groups(&takeovers).len() == affected.len();
        if recovered || Instant::now() >= deadline {
            break takeovers;
        }
        sleep(FAILOVER_POLL).await;
    };

    // The member that did not take a group over would watch it until the limit of the watch, and
    // its watcher would count among the tasks of the next window.
    try_join_all(survivors.iter().map(|node| {
        let mut admin = node.admin();
        async move { admin.disarm_failover(()).await }
    }))
    .await?;

    let moved = count_moved(survivors, phase).await?;
    Ok(FailoverSummary::new(phase, &affected, faulted_ns, &takeovers, moved))
}

/// Have node 1, which is about to fail, record how it rejoins its groups once it returns.
async fn arm_rejoin(node_1: &NodeProcess) -> anyhow::Result<()> {
    let after_ns = clock::monotonic_ns() + REJOIN_ARM_DELAY.as_nanos() as u64;
    node_1.admin().arm_rejoin(pb::ArmRejoinRequest { after_ns }).await?;
    Ok(())
}

/// End the fault of `returning`, node 1, with `end_fault`, and measure how it rejoins the groups,
/// until it has rejoined every group or for [`REJOIN_LIMIT`]. `others` are the nodes that stayed
/// up.
async fn rejoin<F>(
    returning: &NodeProcess,
    others: &[NodeProcess],
    phase: Phase,
    end_fault: F,
) -> anyhow::Result<RejoinSummary>
where
    F: AsyncFnOnce() -> anyhow::Result<()>,
{
    let begin_terms = get_terms(others).await?;

    let returned_ns = clock::monotonic_ns();
    end_fault().await?;

    let deadline = Instant::now() + REJOIN_LIMIT;
    let rejoins = loop {
        let rejoins = returning.admin().get_rejoins(()).await?.into_inner();

        let rejoined = rejoins.rejoins.iter().all(|rejoin| rejoin.rejoined_ns > 0);
        if rejoined || Instant::now() >= deadline {
            break rejoins;
        }
        sleep(REJOIN_POLL).await;
    };
    returning.admin().disarm_rejoin(()).await?;

    let mut end_terms = get_terms(others).await?;
    end_terms.extend(get_terms(std::slice::from_ref(returning)).await?);
    Ok(RejoinSummary::new(
        phase,
        returned_ns,
        &rejoins,
        &begin_terms,
        &end_terms,
    ))
}

/// Return the terms of the members of every group on each of `nodes`.
async fn get_terms(nodes: &[NodeProcess]) -> anyhow::Result<Vec<pb::Terms>> {
    let terms = try_join_all(nodes.iter().map(|node| {
        let mut admin = node.admin();
        async move { admin.get_terms(()).await.map(|response| response.into_inner()) }
    }))
    .await?;
    Ok(terms)
}

/// Count the groups that a surviving node initialized, but no longer leads.
async fn count_moved(survivors: &[NodeProcess], phase: Phase) -> anyhow::Result<usize> {
    let leaders = try_join_all(survivors.iter().map(|node| {
        let mut admin = node.admin();
        async move { admin.get_leaders(()).await.map(|response| (node.id, response.into_inner().leaders)) }
    }))
    .await?;
    let leaders = leaders.into_iter().collect::<BTreeMap<_, _>>();

    let mut moved = 0;
    for (node_id, node_leaders) in &leaders {
        let first_led = (0..phase.groups).filter(|group_id| group_id % phase.nodes == node_id - 1);
        moved += first_led.filter(|group_id| node_leaders.get(*group_id as usize) != Some(node_id)).count();
    }
    Ok(moved)
}

/// Print `line` to stdout at once, so that a run that fails later keeps the lines of its earlier
/// phases.
fn print_line<T>(line: T) -> anyhow::Result<()>
where T: fmt::Display {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{line}")?;
    stdout.flush()?;
    Ok(())
}
