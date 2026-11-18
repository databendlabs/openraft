//! Measure the idle cost of many Raft groups in one process, in the setup of
//! [#2304](https://github.com/databendlabs/openraft/issues/2304).

mod group_network;
mod mode;
mod process_usage;
mod report;
mod stub_followers;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use clap::Parser;
use futures::future::join_all;
use openraft::Config;
use openraft::ServerState;
use openraft_memstore::MemStateMachine;
use openraft_memstore::new_mem_store;

use crate::group_network::GroupNetwork;
use crate::mode::Mode;
use crate::report::Report;
use crate::stub_followers::StubFollowers;

pub(crate) type C = openraft_memstore::TypeConfig;
pub(crate) type Raft = openraft::Raft<C, Arc<MemStateMachine>>;

/// Measure the tasks, memory and CPU of many Raft groups whose leader members share this process.
///
/// The two followers of each group are stubs that grant every vote and acknowledge every
/// AppendEntries request, so the numbers cover the leader side only.
#[derive(Debug, Clone, Parser)]
struct Args {
    /// The number of groups.
    #[arg(long, default_value_t = 1000)]
    groups: u64,

    /// Which timers stay on once every group has elected its leader.
    #[arg(long, value_enum, default_value_t = Mode::Default)]
    mode: Mode,

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

/// Start `args.groups` groups whose leaders are in this process, switch their timers as
/// `args.mode` says, and measure the process.
async fn measure(args: &Args) -> Report {
    let followers = StubFollowers::default();
    let config = Arc::new(Config::default().validate().expect("the default Config is valid"));

    let mut rafts = Vec::new();
    for group_id in 0..args.groups {
        let (log_store, state_machine) = new_mem_store();
        let network = GroupNetwork::new(followers.clone(), group_id);
        let raft = Raft::new(1, config.clone(), network, log_store, state_machine).await.expect("create a group");
        rafts.push(raft);
    }

    let elections = rafts.iter().map(|raft| async move {
        raft.initialize(BTreeSet::from([1, 2, 3])).await.expect("initialize a group");
        let wait = raft.wait(Some(Duration::from_secs(60)));
        wait.state(ServerState::Leader, "elect the leader").await.expect("elect the leader");
    });
    join_all(elections).await;

    // Switch the timers only now. `initialize()` leaves the first election to the election timer,
    // which runs on ticks up to 0.10.0, so `enable_tick: false` in `Config` would elect no leader.
    for raft in &rafts {
        args.mode.apply(raft);
    }

    tokio::time::sleep(Duration::from_secs(args.settle_secs)).await;

    let appends_before = followers.appends();
    let cpu_before = process_usage::cpu_secs();
    let started = Instant::now();

    tokio::time::sleep(Duration::from_secs(args.measure_secs)).await;

    let elapsed = started.elapsed().as_secs_f64();
    let cpu_after = process_usage::cpu_secs();

    Report {
        groups: args.groups,
        mode: args.mode,
        tasks: tokio::runtime::Handle::current().metrics().num_alive_tasks(),
        memory_kib: process_usage::memory_kib(),
        cpu_percent: cpu_before.zip(cpu_after).map(|(before, after)| 100.0 * (after - before) / elapsed),
        appends_per_sec: (followers.appends() - appends_before) as f64 / elapsed,
    }
}

#[cfg(test)]
mod tests {
    use super::Args;
    use super::measure;
    use crate::mode::Mode;

    #[test]
    fn test_leaders_send_heartbeats_to_the_stub_followers() {
        let args = Args {
            groups: 3,
            mode: Mode::Default,
            settle_secs: 0,
            measure_secs: 1,
            workers: 2,
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(args.workers)
            .enable_all()
            .build()
            .unwrap();

        let report = runtime.block_on(measure(&args));

        assert!(report.appends_per_sec > 0.0, "got: {report}");
        assert!(report.tasks >= args.groups as usize, "got: {report}");
    }
}
