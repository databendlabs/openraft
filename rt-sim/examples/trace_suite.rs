//! Prints the scheduling trace of the conformance suite on one `SimRuntime`, then runs
//! `DetsimSuite`.
//!
//! Run it twice in separate processes and compare, including the `DetsimSuite` runtimes:
//!
//! ```text
//! OPENRAFT_RT_SIM_TRACE=/tmp/a-detsim.log cargo run --example trace_suite > /tmp/a-suite.log
//! OPENRAFT_RT_SIM_TRACE=/tmp/b-detsim.log cargo run --example trace_suite > /tmp/b-suite.log
//! diff /tmp/a-suite.log /tmp/b-suite.log && diff /tmp/a-detsim.log /tmp/b-detsim.log
//! ```

#[path = "../tests/common/mod.rs"]
mod common;

use openraft_rt::AsyncRuntime;
use openraft_rt::testing::DetsimSuite;
use openraft_rt_sim::SimRuntime;

fn main() {
    let seed = std::env::args().nth(1).map_or(0, |arg| arg.parse().expect("seed must be a u64"));

    let mut rt = SimRuntime::with_seed(seed);
    rt.record_trace(true);
    rt.block_on(common::run_suite());
    for event in rt.take_trace() {
        println!("{event}");
    }

    DetsimSuite::<SimRuntime>::test_all();
}
