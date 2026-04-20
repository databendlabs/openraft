//! Leader Completeness (Paper §3.6.3, Vanlightly `LeaderHasAllAckedValues`).
//!
//! > If a log entry is committed in a given term, then that entry will be
//! > present in the logs of the leaders for all higher-numbered terms.
//!
//! # Operational interpretation
//!
//! In openraft a **confirmed leader** is a node whose persisted vote is
//! granted-by-quorum (`vote.is_committed()`) and points to itself. This
//! captures both the current leader and every past leader (since a node
//! only clears its vote by adopting a higher one — so once confirmed,
//! the log invariant must hold forever).
//!
//! `CLeaderId`'s total order does the right thing in both leader-id modes.

use std::collections::HashMap;

use openraft::vote::RaftLeaderId;

use super::violation::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::typ::LogId;
use crate::typ::NodeId;

/// For every confirmed leader, require its log to contain all entries
/// committed by any *earlier* leader.
pub fn check(snapshots: &[(NodeId, FullNodeSnapshot)], violations: &mut Vec<InvariantViolation>) {
    let committed = collect_committed_entries(snapshots);

    for (node_id, s) in snapshots {
        if !s.raft.vote.is_committed() {
            continue;
        }
        let vote_lid = s.raft.vote.leader_id();
        if vote_lid.node_id() != node_id {
            continue;
        }

        let leader_clid = vote_lid.to_committed();
        let purged_idx = s.raft.log_id_list.purged().map(|id| id.index()).unwrap_or(0);

        for (idx, committed_id) in &committed {
            // Only care about entries committed by *earlier* leaders.
            if committed_id.committed_leader_id() >= &leader_clid {
                continue;
            }

            let has_it = match s.raft.log_id_list.get(*idx) {
                Some(lid) => lid.committed_leader_id() == committed_id.committed_leader_id(),
                // Not in the log but within the purged prefix: fine, we trust
                // the snapshot contains it.
                None => *idx <= purged_idx,
            };

            if !has_it {
                violations.push(InvariantViolation::LeaderMissingCommitted {
                    leader: *node_id,
                    leader_id: leader_clid,
                    missing_index: *idx,
                    committed_by: *committed_id.committed_leader_id(),
                });
            }
        }
    }
}

/// Build a `index -> LogId` map of every committed entry any node currently reports.
///
/// If multiple nodes report the same index, we keep the first — any divergence
/// is caught separately by [`log_matching::check`](super::log_matching::check).
fn collect_committed_entries(snapshots: &[(NodeId, FullNodeSnapshot)]) -> HashMap<u64, LogId> {
    let mut committed: HashMap<u64, LogId> = HashMap::new();
    for (_, s) in snapshots {
        let Some(c) = s.raft.committed else { continue };

        // Include the purged entry (`LogIdList::get(purged.index)` returns it).
        let first = s.raft.log_id_list.purged().map(|id| id.index()).unwrap_or(0);
        for idx in first..=c.index() {
            if let Some(lid) = s.raft.log_id_list.get(idx) {
                committed.entry(idx).or_insert(lid);
            }
        }
    }
    committed
}
