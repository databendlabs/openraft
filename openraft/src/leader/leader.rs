use std::collections::BTreeSet;

use crate::progress::Progress;
use crate::progress::VecProgress;
use crate::quorum::QuorumSet;
use crate::LogId;
use crate::NodeId;

/// Leader data.
///
/// Openraft leader is the combination of Leader and Candidate in original raft.
/// A node becomes Leader at once when starting election, although at this time, it can not propose any new log, because
/// its `vote` has not yet been granted by a quorum. I.e., A leader without commit vote is a Candidate in original raft.
///
/// When the leader's vote is committed, i.e., granted by a quorum,
/// `Vote.committed` is set to true.
/// Then such a leader is the Leader in original raft.
///
/// By combining candidate and leader into one stage, openraft does not need to lose leadership when a higher
/// `leader_id`(roughly the `term` in original raft) is seen.
/// But instead it will be able to upgrade its `leader_id` without losing leadership.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct Leader<NID: NodeId, QS: QuorumSet<NID>> {
    /// Which nodes have granted the the vote of this node.
    pub(crate) vote_granted_by: BTreeSet<NID>,

    /// Tracks the replication progress and committed index
    pub(crate) progress: VecProgress<NID, Option<LogId<NID>>, QS>,
}

impl<NID, QS> Leader<NID, QS>
where
    NID: NodeId,
    QS: QuorumSet<NID> + 'static,
{
    pub(crate) fn new(quorum_set: QS, learner_ids: impl Iterator<Item = NID>) -> Self {
        Self {
            vote_granted_by: BTreeSet::new(),
            progress: VecProgress::new(quorum_set, learner_ids),
        }
    }

    /// Update that a node has granted the vote.
    pub(crate) fn grant_vote_by(&mut self, target: NID) {
        self.vote_granted_by.insert(target);
    }

    /// Return if a quorum of `membership` has granted it.
    pub(crate) fn is_vote_granted(&self) -> bool {
        let qs = self.progress.quorum_set();
        qs.is_quorum(self.vote_granted_by.iter())
    }
}
