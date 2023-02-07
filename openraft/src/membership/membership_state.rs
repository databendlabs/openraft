use std::error::Error;
use std::sync::Arc;

use crate::less_equal;
use crate::node::Node;
use crate::validate::Validate;
use crate::EffectiveMembership;
use crate::NodeId;

/// The state of membership configs a raft node needs to know.
///
/// A raft node needs to store at most 2 membership config log:
/// - The first(committed) one must have been committed, because (1): raft allows to propose new membership only when
///   the previous one is committed.
/// - The second(effective) may be committed or not.
///
/// From (1) we have:
/// (2) there is at most one outstanding, uncommitted membership log. On
/// either leader or follower, the second last one must have been committed.
/// A committed log must be consistent with the leader.
///
/// (3) By raft design, the last membership takes effect.
///
/// When handling append-entries RPC:
/// (4) a raft follower will delete logs that are inconsistent with the leader.
///
/// From (3) and (4), a follower needs to revert the effective membership to the previous one.
///
/// From (2), a follower only need to revert at most one membership log.
///
/// Thus a raft node will only need to store at most two recent membership logs.
#[derive(Debug, Clone, Default)]
#[derive(PartialEq, Eq)]
pub struct MembershipState<NID, N>
where
    NID: NodeId,
    N: Node,
{
    committed: Arc<EffectiveMembership<NID, N>>,

    // Using `Arc` because the effective membership will be copied to RaftMetrics frequently.
    effective: Arc<EffectiveMembership<NID, N>>,
}

impl<NID, N> MembershipState<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) fn new(
        committed: Arc<EffectiveMembership<NID, N>>,
        effective: Arc<EffectiveMembership<NID, N>>,
    ) -> Self {
        Self { committed, effective }
    }

    pub(crate) fn is_voter(&self, id: &NID) -> bool {
        self.effective.membership.is_voter(id)
    }

    // ---

    pub(crate) fn set_committed(&mut self, c: Arc<EffectiveMembership<NID, N>>) {
        self.committed = c
    }

    pub(crate) fn set_effective(&mut self, e: Arc<EffectiveMembership<NID, N>>) {
        self.effective = e
    }

    pub fn committed(&self) -> &Arc<EffectiveMembership<NID, N>> {
        &self.committed
    }

    pub fn effective(&self) -> &Arc<EffectiveMembership<NID, N>> {
        &self.effective
    }
}

impl<NID, N> Validate for MembershipState<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        less_equal!(self.committed.log_id, self.effective.log_id);
        Ok(())
    }
}
