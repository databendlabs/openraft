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
//! # The term an entry is committed in
//!
//! It is not the term of the entry's log id: a leader commits entries of
//! earlier leaders along with its own (Paper §3.6.2), so an entry written in
//! term 2 may first commit in term 4, and the leader of term 3 need not hold
//! it. The checker remembers, for each committed index, the vote of the first
//! node seen to report it committed. A node learns of a commit from the
//! leader that made it, or a later one, and adopts that leader's vote first
//! (vote-first protocol), so the vote is no older than the committing leader:
//! requiring the entry only of leaders after it can miss a violation but
//! never invents one.
//!
//! `CLeaderId`'s total order does the right thing in both leader-id modes.

use std::collections::BTreeMap;

use openraft::vote::RaftLeaderId;

use super::CLeaderId;
use super::violation::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::typ::LogId;
use crate::typ::NodeId;

/// Every committed entry seen so far, with a leader no older than the one
/// that committed it.
#[derive(Default)]
pub struct CommitHistory {
    committed: BTreeMap<u64, (LogId, CLeaderId)>,
}

impl CommitHistory {
    /// Record newly committed entries, then require every confirmed leader's
    /// log to contain each one committed before its term.
    pub fn check_and_record(
        &mut self,
        snapshots: &[(NodeId, FullNodeSnapshot)],
        violations: &mut Vec<InvariantViolation>,
    ) {
        self.record(snapshots);

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

            for (idx, (committed_id, committed_by)) in self.committed.range(purged_idx..) {
                // Only entries committed by *earlier* leaders must be present.
                if *committed_by >= leader_clid {
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
                        committed_by: *committed_by,
                    });
                }
            }
        }
    }

    /// Record each committed entry any node reports, with that node's vote,
    /// unless an earlier report recorded it.
    ///
    /// If nodes disagree on an index, the first report stays — any
    /// divergence is caught separately by [`log_matching::check`](super::log_matching::check).
    fn record(&mut self, snapshots: &[(NodeId, FullNodeSnapshot)]) {
        for (_, s) in snapshots {
            let Some(c) = s.raft.local_committed else { continue };
            let by = s.raft.vote.leader_id().to_committed();

            // Include the purged entry (`LogIdList::get(purged.index)` returns it).
            let first = s.raft.log_id_list.purged().map(|id| id.index()).unwrap_or(0);
            for idx in first..=c.index() {
                if let Some(lid) = s.raft.log_id_list.get(idx) {
                    self.committed.entry(idx).or_insert((lid, by));
                }
            }
        }
    }
}
