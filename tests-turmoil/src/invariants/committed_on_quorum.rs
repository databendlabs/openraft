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
//! false positives. So only entries this leader itself committed are checked,
//! for which the quorum must reside in the current membership: entries of its
//! own term, and entries of earlier terms that its commit index passed while
//! it led. The latter is what openraft [#2118] got wrong: a new leader counted
//! its own unflushed tail of the previous term as matched.
//!
//! [#2118]: https://github.com/databendlabs/openraft/issues/2118

use std::collections::BTreeMap;

use openraft::alias::LogIdListOf;
use openraft::vote::RaftLeaderId;

use super::CLeaderId;
use super::violation::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::typ::NodeId;
use crate::typ::TypeConfig;

/// Where each leader's commit index stood when the checker first saw it
/// lead: `(incarnation, leader id, committed index)`. Past it, the leader
/// committed the entries itself.
#[derive(Default)]
pub struct LeaderCommits {
    first_seen: BTreeMap<NodeId, (u64, CLeaderId, u64)>,
}

impl LeaderCommits {
    /// On every current leader, if the newest committed entry is one the
    /// leader committed itself, verify a quorum of voters have that entry in
    /// durable log storage.
    pub fn check_and_record(
        &mut self,
        snapshots: &[(NodeId, FullNodeSnapshot)],
        durable_logs: &BTreeMap<NodeId, LogIdListOf<TypeConfig>>,
        violations: &mut Vec<InvariantViolation>,
    ) {
        for (leader_id, s) in snapshots {
            if !s.raft.state.is_leader() {
                continue;
            }
            let Some(committed) = &s.raft.local_committed else {
                continue;
            };

            let leader_clid = s.raft.vote.leader_id().to_committed();
            let seen = (s.incarnation, leader_clid);
            let base = match self.first_seen.get(leader_id) {
                Some((incarnation, clid, base)) if (*incarnation, *clid) == seen => *base,
                _ => {
                    self.first_seen.insert(*leader_id, (s.incarnation, leader_clid, committed.index()));
                    committed.index()
                }
            };

            let own_term = *committed.committed_leader_id() == leader_clid;
            if own_term || committed.index() > base {
                check_leader(*leader_id, s, snapshots, durable_logs, violations);
            }
        }
    }
}

/// Verify a quorum of `s`'s voters have its newest committed entry in durable
/// log storage.
fn check_leader(
    leader_id: NodeId,
    s: &FullNodeSnapshot,
    snapshots: &[(NodeId, FullNodeSnapshot)],
    durable_logs: &BTreeMap<NodeId, LogIdListOf<TypeConfig>>,
    violations: &mut Vec<InvariantViolation>,
) {
    let by_id: BTreeMap<NodeId, &FullNodeSnapshot> = snapshots.iter().map(|(id, s)| (*id, s)).collect();

    let Some(committed) = &s.raft.local_committed else {
        return;
    };
    let expected = committed.committed_leader_id();

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
            return;
        };
        if joint_config.len() == 1 {
            return;
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
                leader: leader_id,
                index: idx,
                voters,
                matching,
            });
        }
    }
}
