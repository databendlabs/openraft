use std::error::Error;
use std::fmt;
use std::sync::Arc;

use openraft_macros::since;
use validit::Validate;

use crate::LogIdOptionExt;

mod change_handler;
#[cfg(test)]
mod change_handler_test;
mod committed_membership_transition;
#[cfg(test)]
mod membership_state_test;

pub(crate) use change_handler::ChangeHandler;
pub(crate) use committed_membership_transition::CommittedMembershipTransition;

use crate::log_id::LogId;
use crate::membership::StoredMembership;
use crate::node::Node;
use crate::node::NodeId;
use crate::vote::RaftCommittedLeaderId;

/// The state of membership configs a raft node needs to know.
///
/// `MembershipState` is a minimal store of the membership log and the membership state machine:
/// - `committed` is the membership state machine, i.e., the last committed membership;
/// - `effective` is the last membership log, and thus plays the role of a log entry: the last seen
///   membership, committed or not.
///
/// A raft node needs to store at most 2 membership config logs:
/// - The first(committed) one must have been committed, because (1): raft allows proposing new
///   membership only when the previous one is committed.
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
/// From (2), a follower only needs to revert at most one membership log.
///
/// Thus, a raft node will only need to store at most two recent membership logs.
#[since(
    version = "0.10.0",
    change = "from `MembershipState<C>` to `MembershipState<CLID, NID, N>`"
)]
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
pub struct MembershipState<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    committed: Arc<StoredMembership<CLID, NID, N>>,

    // Using `Arc` because the effective membership will be copied to RaftMetrics frequently.
    effective: Arc<StoredMembership<CLID, NID, N>>,
}

impl<CLID, NID, N> Default for MembershipState<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    fn default() -> Self {
        Self {
            committed: Arc::new(StoredMembership::default()),
            effective: Arc::new(StoredMembership::default()),
        }
    }
}

impl<CLID, NID, N> fmt::Display for MembershipState<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "MembershipState{{committed: {}, effective: {}}}",
            self.committed, self.effective
        )
    }
}

impl<CLID, NID, N> MembershipState<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    pub(crate) fn new(
        committed: Arc<StoredMembership<CLID, NID, N>>,
        effective: Arc<StoredMembership<CLID, NID, N>>,
    ) -> Self {
        Self { committed, effective }
    }

    /// Return true if the given node id is either a voter or a learner.
    pub(crate) fn contains(&self, id: &NID) -> bool {
        self.effective.membership().contains(id)
    }

    /// Check if the given `NodeId` exists and is a voter.
    pub(crate) fn is_voter(&self, id: &NID) -> bool {
        self.effective.membership().is_voter(id)
    }

    /// Commit the effective membership config if `committed_log_id` is greater than or equal to
    /// its log id.
    ///
    /// Committing replaces `self.committed`(the membership state machine) with
    /// `self.effective`(the last membership log).
    ///
    /// If the committed membership config changes, it returns the committed log ids before and
    /// after the update; otherwise it returns `None`, e.g., when the effective membership is
    /// already committed.
    pub(crate) fn commit(
        &mut self,
        committed_log_id: &Option<LogId<CLID>>,
    ) -> Option<CommittedMembershipTransition<CLID>> {
        let current = self.committed.log_id().clone();
        let last = self.effective().log_id().clone();

        // `current == last` means the effective membership is already committed.
        if committed_log_id >= &last && current < last {
            debug_assert!(committed_log_id.index() >= last.index());
            self.committed = self.effective.clone();

            // `current < last` implies `last` is not `None`.
            return Some(CommittedMembershipTransition {
                before: current,
                after: last.unwrap(),
            });
        }

        None
    }

    /// Install the membership config carried by a snapshot.
    ///
    /// A snapshot stores the last membership applied to its state machine, so this membership is
    /// committed. It updates `self.committed`(the membership state machine), and may also update
    /// `self.effective`(the last membership log), each only when the snapshot is newer than the
    /// corresponding local membership.
    pub(crate) fn install_membership_snapshot(&mut self, membership_snapshot: Arc<StoredMembership<CLID, NID, N>>) {
        // The local effective membership may conflict with the leader.
        // Thus, it has to compare by log-index, e.g.:
        //   membership.log_id       = (10, 5);
        //   local_effective.log_id = (2, 10);
        if membership_snapshot.log_id().index() >= self.effective.log_id().index() {
            // The effective may override by a new leader with a different one.
            self.effective = membership_snapshot.clone()
        }

        #[allow(clippy::collapsible_if)]
        if cfg!(debug_assertions) {
            if membership_snapshot.log_id() == self.committed.log_id() {
                debug_assert_eq!(
                    membership_snapshot.membership(),
                    self.committed.membership(),
                    "the same log id implies the same membership"
                );
            }
        }

        // same log id implies the same membership,
        // so it only needs to compare log id.
        if membership_snapshot.log_id() > self.committed.log_id() {
            self.committed = membership_snapshot
        }
    }

    /// Append a membership config `m`.
    ///
    /// It assumes `self.effective` does not conflict with the leader's log, i.e.:
    /// - Leader appends a new membership,
    /// - Or a follower has confirmed preceding logs matches the leaders' and appends membership
    ///   received from the leader.
    pub(crate) fn append(&mut self, m: Arc<StoredMembership<CLID, NID, N>>) {
        debug_assert!(
            m.log_id() > self.effective.log_id(),
            "new membership has to have a greater log_id"
        );
        debug_assert!(
            m.log_id().index() > self.effective.log_id().index(),
            "new membership has to have a greater index"
        );

        // Openraft allows at most only one non-committed membership config.
        // If there is another new config, self.effective must have been committed.
        self.committed = self.effective.clone();
        self.effective = m;
    }

    /// Truncate membership state if the log is truncated since `since`(inclusive).
    ///
    /// It returns the updated effective membership config if it is changed.
    ///
    /// It will reset `self.effective` to `self.committed`. Only the effective could be truncated
    /// when a new leader tries to truncate follower logs that the leader does not have.
    ///
    /// If the effective membership is from a conflicting log,
    /// the membership state has to revert to the last committed membership config.
    /// See: [Effective-membership](crate::docs::data::effective_membership)
    ///
    /// ```text
    /// committed_membership, ... since, ... effective_membership // log
    /// ^                                    ^
    /// |                                    |
    /// |                                    last membership      // before deleting since..
    /// last membership                                           // after  deleting since..
    /// ```
    pub(crate) fn truncate(&mut self, since: u64) -> Option<Arc<StoredMembership<CLID, NID, N>>> {
        debug_assert!(
            since >= self.committed().log_id().next_index(),
            "committed log should never be truncated: committed membership cannot conflict with the leader"
        );

        if Some(since) <= self.effective().log_id().index() {
            tracing::debug!(
                "effective membership is in conflicting logs, revert it to last committed: effective: {}, committed: {}",
                self.effective(),
                self.committed()
            );

            self.effective = self.committed.clone();
            return Some(self.effective.clone());
        }
        None
    }

    // This method is only used by tests
    #[cfg(test)]
    pub(crate) fn set_effective(&mut self, e: Arc<StoredMembership<CLID, NID, N>>) {
        self.effective = e
    }

    /// Returns a reference to the last committed membership config.
    ///
    /// A committed membership config may or may not be the same as the effective one.
    pub fn committed(&self) -> &Arc<StoredMembership<CLID, NID, N>> {
        &self.committed
    }

    /// Returns a reference to the presently effective membership config.
    ///
    /// In openraft the last seen membership config, whether committed or not, is the effective
    /// one.
    ///
    /// A committed membership config may or may not be the same as the effective one.
    pub fn effective(&self) -> &Arc<StoredMembership<CLID, NID, N>> {
        &self.effective
    }

    pub(crate) fn change_handler(&self) -> ChangeHandler<'_, CLID, NID, N> {
        ChangeHandler { state: self }
    }
}

impl<CLID, NID, N> Validate for MembershipState<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        validit::less_equal!(self.committed.log_id(), self.effective.log_id());
        validit::less_equal!(self.committed.log_id().index(), self.effective.log_id().index());
        Ok(())
    }
}
