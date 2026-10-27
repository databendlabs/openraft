//! A Raft cluster for type configs whose membership is not the default `Membership`.
//!
//! [`RaftRouter`](crate::fixtures::RaftRouter) runs only the memstore type config, so it cannot
//! run another membership type. [`TestCluster`] runs any [`TestConfig`].

mod change_scenario;
mod log_store;
mod minimal_quorums;
mod network;
mod quorum_scenario;
mod state_machine;
mod test_cluster;

pub use change_scenario::ChangeScenario;
pub use minimal_quorums::MinimalQuorums;
pub use minimal_quorums::MinimalQuorumsMembership;
pub use quorum_scenario::QuorumScenario;
pub use test_cluster::TestCluster;
pub use test_cluster::TestConfig;
pub use test_cluster::cluster_config;
