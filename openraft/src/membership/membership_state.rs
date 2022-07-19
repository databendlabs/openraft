use std::sync::Arc;

use crate::EffectiveMembership;
use crate::NodeId;

/// The state of membership configs a raft node needs to know.
///
/// A raft node needs to store at most 2 membership config log:
// - The first(committed) one must have been committed, because (1): raft allows to propose new membership only when the
//   previous one is committed.
// - The second(effective) may be committed or not.
//
// By (1) we have:
// (2) there is at most one outstanding, uncommitted membership log. On
// either leader or follower, the second last one must have been committed.
// A committed log must be consistent with the leader.
//
// (3) By raft design, the last membership takes effect.
//
// When handling append-entries RPC:
// (4) a raft follower will delete logs that are inconsistent with the leader.
//
// By (3) and (4), a follower needs to revert the effective membership to the previous one.
//
// By (2), a follower only need to revert at most one membership log.
//
// Thus a raft node will only need to store at most two recent membership logs.
#[derive(Debug, Clone, Default)]
#[derive(PartialEq, Eq)]
pub struct MembershipState<NID: NodeId> {
    pub committed: Arc<EffectiveMembership<NID>>,

    // Using `Arc` because the effective membership will be copied to RaftMetrics frequently.
    pub effective: Arc<EffectiveMembership<NID>>,
}

impl<NID: NodeId> MembershipState<NID> {
    pub(crate) fn is_voter(&self, id: &NID) -> bool {
        self.effective.membership.is_voter(id)
    }
}
