//! Election Safety (Paper §3.6.3): at most one leader per [`CLeaderId`].
//!
//! - In standard Raft (`leader_id_std`) `CLeaderId == term`, so this reduces to the textbook "at
//!   most one leader per term".
//! - In `leader_id_adv` mode (openraft's default) `CLeaderId == (term, node_id)`, so two nodes
//!   leading in the same term with different `node_id`s is legitimate and not flagged.

use std::collections::BTreeMap;

use openraft::vote::RaftLeaderId;

use super::CLeaderId;
use super::violation::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::typ::NodeId;

/// Group every `state == Leader` node by its committed leader id, flag groups > 1.
pub fn check(snapshots: &[(NodeId, FullNodeSnapshot)], violations: &mut Vec<InvariantViolation>) {
    // `CLeaderId` is `Ord` (required by `RaftCommittedLeaderId`) but not `Hash`,
    // so we use a `BTreeMap` to group leaders by their committed leader id.
    let mut by_leader_id: BTreeMap<CLeaderId, Vec<NodeId>> = BTreeMap::new();

    for (node_id, s) in snapshots {
        if s.raft.state.is_leader() {
            let clid = s.raft.vote.leader_id().to_committed();
            by_leader_id.entry(clid).or_default().push(*node_id);
        }
    }

    for (leader_id, nodes) in by_leader_id {
        if nodes.len() > 1 {
            violations.push(InvariantViolation::DuplicateLeader { leader_id, nodes });
        }
    }
}
