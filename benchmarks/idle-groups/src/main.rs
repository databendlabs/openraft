//! Measure the idle cost of many Raft groups in one process, in the setup of
//! [#2304](https://github.com/databendlabs/openraft/issues/2304).

mod core_counts;
mod counting_alloc;
mod followers;
mod group_network;
mod local_members;
mod mode;
mod process_usage;
mod report;
mod stub_followers;

use std::collections::BTreeSet;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use clap::Parser;
use futures::future::join_all;
use openraft::Config;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft_memstore::MemStateMachine;
use openraft_memstore::new_mem_store;
use openraft_multi::GroupRouter;

use crate::core_counts::CoreCounts;
use crate::counting_alloc::CountingAlloc;
use crate::followers::Followers;
use crate::group_network::GroupNetwork;
use crate::local_members::LocalMembers;
use crate::mode::Mode;
use crate::report::Report;
use crate::stub_followers::StubFollowers;

pub(crate) type C = openraft_memstore::TypeConfig;
pub(crate) type Raft = openraft::Raft<C, Arc<MemStateMachine>>;

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc::new();

/// Measure the tasks, memory and CPU of many Raft groups whose members share this process.
///
/// By default the two followers of each group are stubs that grant every vote and acknowledge
/// every AppendEntries request, so the numbers cover the leader members only.
#[derive(Debug, Clone, Parser)]
struct Args {
    /// The number of groups.
    #[arg(long, default_value_t = 1000)]
    groups: u64,

    /// What stands in for the two followers of each group.
    #[arg(long, value_enum, default_value_t = Followers::Stub)]
    followers: Followers,

    /// Which timers stay on once every group has elected its leader.
    #[arg(long, value_enum, default_value_t = Mode::Default)]
    mode: Mode,

    /// Count heap allocations, for the `heap_mib` and `allocs/s` fields. Counting adds CPU.
    #[arg(long)]
    count_allocs: bool,

    /// Seconds to wait after switching the timers, before measuring.
    #[arg(long, default_value_t = 10)]
    settle_secs: u64,

    /// Seconds to measure for.
    #[arg(long, default_value_t = 30)]
    measure_secs: u64,

    /// The worker threads of the tokio runtime.
    #[arg(long, default_value_t = 4)]
    workers: usize,
}

fn main() {
    let args = Args::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(args.workers)
        .enable_all()
        .build()
        .expect("build the tokio runtime");

    let report = runtime.block_on(measure(&args));
    println!("{report}");

    // Exit without shutting the groups down: with thousands of groups that takes longer than the
    // measurement, and it measures nothing.
    std::process::exit(0);
}

/// Start `args.groups` groups, elect a leader in each, switch the timers of every member as
/// `args.mode` says, and measure the process.
async fn measure(args: &Args) -> Report {
    if args.count_allocs {
        ALLOC.start();
    }

    let appends = Arc::new(AtomicU64::new(0));
    let groups = match args.followers {
        Followers::Stub => new_groups(args.groups, 1, StubFollowers::new(appends.clone())).await,
        Followers::Local => {
            let router = LocalMembers::new(appends.clone());
            let groups = new_groups(args.groups, 3, router.clone()).await;
            router.set_members(&groups);
            groups
        }
    };

    // Only node 1 is initialized, so the others cannot campaign before it replicates the
    // membership to them: node 1 becomes the leader.
    let elections = groups.iter().map(|group| async move {
        let raft = &group[0];
        raft.initialize(BTreeSet::from([1, 2, 3])).await.expect("initialize a group");
        let wait = raft.wait(Some(Duration::from_secs(60)));
        wait.state(ServerState::Leader, "elect the leader").await.expect("elect the leader");
    });
    join_all(elections).await;

    // Switch the timers only now. `initialize()` leaves the first election to the election timer,
    // which runs on ticks up to 0.10.0, so `enable_tick: false` in `Config` would elect no leader.
    for raft in groups.iter().flatten() {
        args.mode.apply(raft);
    }

    tokio::time::sleep(Duration::from_secs(args.settle_secs)).await;

    // Read the counters outside the CPU window, so that reading them adds no CPU to it.
    let counts_before = CoreCounts::read(&groups).await;
    let appends_before = appends.load(Ordering::Relaxed);
    let terms_before = highest_terms(&groups);
    let allocs_before = ALLOC.allocs();
    let cpu_before = process_usage::cpu_secs();
    let started = Instant::now();

    tokio::time::sleep(Duration::from_secs(args.measure_secs)).await;

    let elapsed = started.elapsed().as_secs_f64();
    let cpu_after = process_usage::cpu_secs();
    let allocs_after = ALLOC.allocs();
    let counts_after = CoreCounts::read(&groups).await;

    let counts = counts_before.zip(counts_after).map(|(before, after)| after.since(&before));
    let per_sec = |count: u64| count as f64 / elapsed;

    Report {
        groups: args.groups,
        followers: args.followers,
        mode: args.mode,
        tasks: tokio::runtime::Handle::current().metrics().num_alive_tasks(),
        memory_kib: process_usage::memory_kib(),
        heap_kib: ALLOC.live_bytes().map(|bytes| bytes / 1024),
        cpu_percent: cpu_before.zip(cpu_after).map(|(before, after)| 100.0 * (after - before) / elapsed),
        allocs_per_sec: allocs_before.zip(allocs_after).map(|(before, after)| (after - before) as f64 / elapsed),
        appends_per_sec: (appends.load(Ordering::Relaxed) - appends_before) as f64 / elapsed,
        elections: highest_terms(&groups) - terms_before,
        fires_per_sec: counts.map(|c| per_sec(c.fires)),
        rearms_per_sec: counts.map(|c| per_sec(c.rearms)),
        metrics_per_sec: counts.map(|c| per_sec(c.metrics)),
        data_metrics_per_sec: counts.map(|c| per_sec(c.data_metrics)),
        server_metrics_per_sec: counts.map(|c| per_sec(c.server_metrics)),
    }
}

/// Create `groups` groups that connect through `router`. Each has `members` members in this
/// process, with node ids from 1 up.
async fn new_groups<R>(groups: u64, members: u64, router: R) -> Vec<Vec<Raft>>
where R: GroupRouter<C, u64, SnapshotData = Cursor<Vec<u8>>> {
    let config = Arc::new(Config::default().validate().expect("the default Config is valid"));

    let mut rafts = Vec::new();
    for group_id in 0..groups {
        let mut group = Vec::new();
        for node_id in 1..=members {
            let (log_store, state_machine) = new_mem_store();
            let network = GroupNetwork::new(router.clone(), group_id);
            let raft = Raft::new(node_id, config.clone(), network, log_store, state_machine).await;
            group.push(raft.expect("create a member"));
        }
        rafts.push(group);
    }
    rafts
}

/// The highest term that a member of each group has seen, summed over `groups`.
fn highest_terms(groups: &[Vec<Raft>]) -> u64 {
    let highest_term = |group: &Vec<Raft>| group.iter().map(|raft| raft.metrics().borrow_watched().current_term).max();
    groups.iter().filter_map(highest_term).sum()
}

#[cfg(test)]
mod tests {
    use super::Args;
    use super::measure;
    use crate::followers::Followers;
    use crate::mode::Mode;
    use crate::report::Report;

    fn measure_briefly(followers: Followers, mode: Mode) -> Report {
        let args = Args {
            groups: 3,
            followers,
            mode,
            count_allocs: true,
            settle_secs: 0,
            measure_secs: 1,
            workers: 2,
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(args.workers)
            .enable_all()
            .build()
            .unwrap();

        runtime.block_on(measure(&args))
    }

    #[test]
    fn test_leaders_send_heartbeats_to_the_stub_followers() {
        let report = measure_briefly(Followers::Stub, Mode::Default);

        assert!(report.appends_per_sec > 0.0, "got: {report}");
        assert!(report.tasks >= 3, "got: {report}");
        assert!(report.heap_kib > Some(0), "got: {report}");
        assert!(report.allocs_per_sec > Some(0.0), "got: {report}");

        // Each heartbeat fires the heartbeat timer, re-arms it for the next one, and changes the
        // quorum-acknowledged time of the data metrics.
        #[cfg(feature = "runtime-stats")]
        {
            assert!(report.fires_per_sec > Some(0.0), "got: {report}");
            assert!(report.rearms_per_sec > Some(0.0), "got: {report}");
            assert!(report.metrics_per_sec > Some(0.0), "got: {report}");
            assert!(report.data_metrics_per_sec > Some(0.0), "got: {report}");
        }

        #[cfg(not(feature = "runtime-stats"))]
        assert_eq!(None, report.fires_per_sec, "got: {report}");
    }

    #[test]
    fn test_heartbeats_keep_the_leaders_of_local_followers() {
        let report = measure_briefly(Followers::Local, Mode::Default);

        assert!(report.appends_per_sec > 0.0, "got: {report}");
        assert_eq!(0, report.elections, "got: {report}");

        // Each heartbeat a follower receives changes the last leader contact of its server
        // metrics.
        #[cfg(feature = "runtime-stats")]
        assert!(report.server_metrics_per_sec > Some(0.0), "got: {report}");
    }

    #[test]
    fn test_local_followers_campaign_without_heartbeats() {
        let report = measure_briefly(Followers::Local, Mode::NoHeartbeat);

        assert!(report.elections > 0, "got: {report}");
    }
}
