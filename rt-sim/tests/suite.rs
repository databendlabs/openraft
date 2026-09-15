//! `openraft_rt::testing::Suite` and `DetsimSuite` against `SimRuntime`, exactly as the other
//! runtimes run them.

use openraft_rt::testing::DetsimSuite;
use openraft_rt::testing::Suite;
use openraft_rt_sim::SimRuntime;

#[test]
fn test_sim_rt() {
    Suite::<SimRuntime>::test_all();
}

#[test]
fn detsim_suite() {
    DetsimSuite::<SimRuntime>::test_all();
}
