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

use std::collections::HashMap;

use openraft::vote::RaftLeaderId;

use super::violation::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::typ::NodeId;

/// On every current leader, if the newest committed entry is the leader's own,
/// verify a quorum of voters have that entry in their log.
pub fn check(snapshots: &[(NodeId, FullNodeSnapshot)], violations: &mut Vec<InvariantViolation>) {
    let by_id: HashMap<NodeId, &FullNodeSnapshot> = snapshots.iter().map(|(id, s)| (*id, s)).collect();

    for (leader_id, s) in snapshots {
        if !s.raft.state.is_leader() {
            continue;
        }
        let Some(committed) = &s.raft.committed else {
            continue;
        };

        let expected = committed.committed_leader_id();
        if *expected != s.raft.vote.leader_id().to_committed() {
            continue;
        }

        let idx = committed.index();
        let has_entry = |v: &NodeId| -> bool {
            let Some(vs) = by_id.get(v) else { return false };
            vs.raft.log_id_list.get(idx).is_some_and(|lid| lid.committed_leader_id() == expected)
        };

        for voter_set in s.raft.membership_config.membership().get_joint_config() {
            let voters: Vec<NodeId> = voter_set.iter().copied().collect();
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
