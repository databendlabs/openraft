//! The write quorums of every membership type the cluster can run, in one shape.

use std::collections::BTreeSet;

use openraft::Membership;
use openraft::membership::RaftMembership;
use openraft::membership::ThresholdMembership;

use crate::typ::Node;
use crate::typ::NodeId;

/// Each config's voters, and how many of them must store an entry to commit it.
///
/// The configs come in the order of the membership: the oldest config first, and two or more
/// configs only during a joint change.
pub trait WriteConfigs {
    fn write_configs(&self) -> Vec<(BTreeSet<NodeId>, usize)>;
}

/// A config of the default `Membership` commits through a majority of its voters.
impl WriteConfigs for Membership<NodeId, Node> {
    fn write_configs(&self) -> Vec<(BTreeSet<NodeId>, usize)> {
        let mut configs = Vec::new();
        for voters in self.get_joint_config() {
            let write_size = voters.len() / 2 + 1;
            configs.push((voters.clone(), write_size));
        }
        configs
    }
}

/// A config of `ThresholdMembership` commits through any `write_size` of its voters.
impl WriteConfigs for ThresholdMembership<NodeId, Node> {
    fn write_configs(&self) -> Vec<(BTreeSet<NodeId>, usize)> {
        let mut configs = Vec::new();
        for rule in self.write_quorum_set().rules() {
            let voters = rule.id_set().as_ref().clone();
            let write_size = rule.quorum_size() as usize;
            configs.push((voters, write_size));
        }
        configs
    }
}
