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
use crate::scenario::Scenario;
use crate::window_summary::WindowSummary;

/// How long the `failover` phase waits for the groups of the failed node to recover.
const FAILOVER_LIMIT: Duration = Duration::from_secs(30);

/// How often the `failover` phase asks the surviving nodes how far they got.
const FAILOVER_POLL: Duration = Duration::from_millis(20);

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

    /// Seconds of each window of a fault scenario: `baseline` and `degraded`.
    #[arg(long, default_value_t = 10)]
    window_secs: u64,

    /// openraft `Config` flags for every node, after `--`, such as `-- --enable-pre-vote true`.
    #[arg(last = true)]
    raft_args: Vec<String>,
}

/// Start the node processes, run the scenario, and print one line per phase.
pub(crate) fn run(args: RunArgs) -> anyhow::Result<()> {
    raft_config::parse(&args.raft_args).context("parse the openraft Config flags")?;
    if args.scenario == Scenario::Crash && args.nodes < 3 {
        anyhow::bail!("crash needs --nodes 3 or more, so that the other nodes keep a quorum");
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

            let crashed = nodes.remove(0);
            print_line(crash(crashed, &nodes, phase("failover")).await?)?;

            print_line(measure_window(&nodes, phase("degraded"), args.window_secs, &[1]).await?)?;
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

/// Kill `crashed`, node 1, with `SIGKILL`, and measure how fast `survivors` take over the groups
/// it led.
async fn crash(crashed: NodeProcess, survivors: &[NodeProcess], phase: Phase) -> anyhow::Result<FailoverSummary> {
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
    crashed.crash()?;

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

    let moved = count_moved(survivors, phase).await?;
    Ok(FailoverSummary::new(phase, &affected, faulted_ns, &takeovers, moved))
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
