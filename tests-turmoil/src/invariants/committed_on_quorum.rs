//! Committed On Quorum (Vanlightly `CommittedEntriesReachMajority`).
//!
//! A current leader's latest committed entry must be present on a voter quorum
//! of its own membership (every sub-config in a joint config).
//!
//! # Why only the *current-term* frontier?
//!
//! Committed entries inherited from a previous leader were replicated on a
//! quorum of the **then-current** membership. Re-checking them against the
//! leader's **current** membership (potentially mid-reconfiguration) produces
//! false positives. By restricting to entries whose committed-leader-id equals
//! this leader's own id, we check only the entries this leader itself
//! committed — for which the quorum must reside in the current membership.

use std::collections::BTreeMap;

use openraft::alias::LogIdListOf;
use openraft::vote::RaftLeaderId;

use super::violation::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::typ::NodeId;
use crate::typ::TypeConfig;

/// On every current leader, if the newest committed entry is the leader's own,
/// verify a quorum of voters have that entry in durable log storage.
pub fn check(
    snapshots: &[(NodeId, FullNodeSnapshot)],
    durable_logs: &BTreeMap<NodeId, LogIdListOf<TypeConfig>>,
    violations: &mut Vec<InvariantViolation>,
) {
    let by_id: BTreeMap<NodeId, &FullNodeSnapshot> = snapshots.iter().map(|(id, s)| (*id, s)).collect();

    for (leader_id, s) in snapshots {
        if !s.raft.state.is_leader() {
            continue;
        }
        let Some(committed) = &s.raft.local_committed else {
            continue;
        };

        let expected = committed.committed_leader_id();
        if *expected != s.raft.vote.leader_id().to_committed() {
            continue;
        }

        let idx = committed.index();
        let has_entry = |v: &NodeId| -> bool {
            match durable_logs.get(v).and_then(|logs| logs.get(idx)) {
                Some(lid) => lid.committed_leader_id() == expected,
                None => by_id.get(v).is_some_and(|vs| vs.sm.last_applied.is_some_and(|lid| lid.index() >= idx)),
            }
        };

        let membership = s.raft.membership_config.as_ref();
        let joint_config = membership.membership().get_joint_config();
        let membership_log_index = membership.log_id().map(|id| id.index());

        let voter_sets: Vec<Vec<NodeId>> = if membership_log_index.is_some_and(|i| idx < i) {
            let Some(old_config) = joint_config.first() else {
                continue;
            };
            if joint_config.len() == 1 {
                continue;
            }
            vec![old_config.iter().copied().collect()]
        } else {
            joint_config.iter().map(|voter_set| voter_set.iter().copied().collect()).collect()
        };

        for voters in voter_sets {
            let matching: Vec<NodeId> = voters.iter().copied().filter(has_entry).collect();

            // Strict-majority check: `matching` must be > half of `voters`.
            // `matching * 2 <= voters` catches both "less than half" and
            // "exactly half of an even voter count" (not a majority).
            if matching.len() * 2 <= voters.len() {
                violations.push(InvariantViolation::CommittedNotOnQuorum {
                    leader: *leader_id,
                    index: idx,
                    voters,
                    matching,
                });
            }
        }
    }
}
