use std::error::Error;
use std::sync::Arc;

use crate::less_equal;
use crate::node::Node;
use crate::validate::Validate;
use crate::EffectiveMembership;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::MessageSummary;
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

impl<NID, N> MessageSummary<MembershipState<NID, N>> for MembershipState<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn summary(&self) -> String {
        format!(
            "MembershipState{{committed: {}, effective: {}}}",
            self.committed().summary(),
            self.effective().summary()
        )
    }
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

    /// Update membership state if the specified committed_log_id is greater than `self.effective`
    pub(crate) fn commit(&mut self, committed_log_id: &Option<LogId<NID>>) {
        if committed_log_id >= &self.effective().log_id {
            debug_assert!(committed_log_id.index() >= self.effective().log_id.index());
            self.committed = self.effective.clone();
        }
    }

    /// A committed membership log is found and the either of `self.committed` and `self.effective` should be updated if
    /// it is smaller than the new one.
    ///
    /// If `self.effective` changed, it returns a reference to the new one.
    /// If not, it returns None.
    pub(crate) fn update_committed(
        &mut self,
        c: Arc<EffectiveMembership<NID, N>>,
    ) -> Option<Arc<EffectiveMembership<NID, N>>> {
        let mut changed = false;

        // The local effective membership may conflict with the leader.
        // Thus it has to compare by log-index, e.g.:
        //   membership.log_id       = (10, 5);
        //   local_effective.log_id = (2, 10);
        if c.log_id.index() >= self.effective.log_id.index() {
            changed = c.membership != self.effective.membership;

            // The effective may override by a new leader with a different one.
            self.effective = c.clone()
        }

        #[allow(clippy::collapsible_if)]
        if cfg!(debug_assertions) {
            if c.log_id == self.committed.log_id {
                debug_assert_eq!(
                    c.membership, self.committed.membership,
                    "the same log id implies the same membership"
                );
            }
        }

        if c.log_id > self.committed.log_id {
            self.committed = c
        }

        if changed {
            Some(self.effective().clone())
        } else {
            None
        }
    }

    /// Append a membership config `m`.
    ///
    /// It assumes `self.effective` does not conflict with the leader's log, i.e.:
    /// - Leader appends a new membership,
    /// - Or a follower has confirmed preceding logs matches the leaders' and appends membership received from the
    ///   leader.
    pub(crate) fn append(&mut self, m: Arc<EffectiveMembership<NID, N>>) {
        debug_assert!(
            m.log_id > self.effective.log_id,
            "new membership has to have a greater log_id"
        );
        debug_assert!(
            m.log_id.index() > self.effective.log_id.index(),
            "new membership has to have a greater index"
        );

        // Openraft allows at most only one non-committed membership config.
        // If there is another new config, self.effective must have been committed.
        self.committed = self.effective.clone();
        self.effective = m;
    }

    // This method is only used by tests
    #[cfg(test)]
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
        less_equal!(self.committed.log_id.index(), self.effective.log_id.index());
        Ok(())
    }
}
