//! A Raft cluster for type configs whose membership is not the default `Membership`.
//!
//! [`RaftRouter`](crate::fixtures::RaftRouter) runs only the memstore type config, so it cannot
//! run another membership type. [`TestCluster`] runs any [`TestConfig`].

mod log_store;
mod minimal_quorums;
mod network;
mod state_machine;
mod test_cluster;

#[allow(unused_imports)]
pub use minimal_quorums::MinimalQuorums;
#[allow(unused_imports)]
pub use minimal_quorums::MinimalQuorumsMembership;
#[allow(unused_imports)]
pub use test_cluster::TestCluster;
#[allow(unused_imports)]
pub use test_cluster::TestConfig;
#[allow(unused_imports)]
pub use test_cluster::cluster_config;
