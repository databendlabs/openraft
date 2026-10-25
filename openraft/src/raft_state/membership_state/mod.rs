use std::error::Error;
use std::fmt;
use std::sync::Arc;

use openraft_macros::since;
use validit::Validate;

use crate::LogIdOptionExt;
use crate::errors::ChangeMembershipError;
use crate::errors::ErrorSource;
use crate::errors::InProgress;
use crate::errors::MembershipError;
use crate::errors::UnsupportedMembershipTransition;

#[cfg(test)]
mod change_membership_test;
#[cfg(test)]
mod membership_state_test;

use crate::log_id::LogId;
use crate::membership::RaftMembership;
use crate::membership::RaftMembershipEdit;
use crate::membership::RaftMembershipExt;
use crate::membership::StoredMembership;
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
    change = "from `MembershipState<C>` to `MembershipState<CLID, Mem>`"
)]
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
pub struct MembershipState<CLID, Mem>
where
    CLID: RaftCommittedLeaderId,
    Mem: RaftMembership,
{
    committed: Arc<StoredMembership<CLID, Mem>>,

    // Using `Arc` because the effective membership will be copied to RaftMetrics frequently.
    effective: Arc<StoredMembership<CLID, Mem>>,
}

impl<CLID, Mem> Default for MembershipState<CLID, Mem>
where
    CLID: RaftCommittedLeaderId,
    Mem: RaftMembership,
{
    fn default() -> Self {
        Self {
            committed: Arc::new(StoredMembership::default()),
            effective: Arc::new(StoredMembership::default()),
        }
    }
}

impl<CLID, Mem> fmt::Display for MembershipState<CLID, Mem>
where
    CLID: RaftCommittedLeaderId,
    Mem: RaftMembership,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "MembershipState{{committed: {}, effective: {}}}",
            self.committed, self.effective
        )
    }
}

impl<CLID, Mem> MembershipState<CLID, Mem>
where
    CLID: RaftCommittedLeaderId,
    Mem: RaftMembership,
{
    pub(crate) fn new(
        committed: Arc<StoredMembership<CLID, Mem>>,
        effective: Arc<StoredMembership<CLID, Mem>>,
    ) -> Self {
        Self { committed, effective }
    }

    /// Returns whether `id` is a voter or learner in the effective membership.
    pub(crate) fn contains(&self, id: &Mem::NodeId) -> bool {
        self.effective.membership().contains(id)
    }

    /// Returns whether `id` may campaign under the effective membership.
    ///
    /// This uses the latest membership even before its log entry commits.
    /// Election triggers use [`Self::is_campaigner_in_effective_or_committed`] instead.
    pub(crate) fn is_campaigner(&self, id: &Mem::NodeId) -> bool {
        self.effective.membership().is_campaigner(id)
    }

    /// Returns whether `id` is a read voter in the effective membership.
    ///
    /// For joint membership, a voter may belong to any component configuration.
    /// This uses the latest membership even before its log entry commits.
    #[allow(dead_code, reason = "Reserved for read/write quorum separation")]
    pub(crate) fn is_read_voter(&self, id: &Mem::NodeId) -> bool {
        self.effective.membership().is_read_voter(id)
    }

    /// Returns whether `id` is a write voter in the effective membership.
    ///
    /// For joint membership, a voter may belong to any component configuration.
    /// This uses the latest membership even before its log entry commits.
    #[allow(dead_code, reason = "Reserved for read/write quorum separation")]
    pub(crate) fn is_write_voter(&self, id: &Mem::NodeId) -> bool {
        self.effective.membership().is_write_voter(id)
    }

    /// Whether a node may campaign according to its effective or committed membership.
    ///
    /// A removed campaigner may need to regain leadership to finish the membership change.
    /// It remains eligible until the membership change commits, even if retained as a learner.
    /// Voting and quorum checks use only the effective membership.
    pub(crate) fn is_campaigner_in_effective_or_committed(&self, id: &Mem::NodeId) -> bool {
        self.effective.is_campaigner(id) || self.committed.is_campaigner(id)
    }

    /// Builds a new membership configuration by applying changes to the current configuration.
    ///
    /// It returns the target of `change` when `supports_direct_transition_to()` holds from the
    /// effective membership to the target. Otherwise it returns the `joint_with()` of the two,
    /// which needs no check: every read quorum of the joint is a read quorum of both memberships,
    /// and the same holds for write quorums. Both memberships passed
    /// `ensure_read_write_quorums_intersect()`, so on each step to and from the joint, every read
    /// quorum of one side intersects every write quorum of the other side.
    ///
    /// E.g.(`cicj` is a joint membership of `ci` and `cj`, and for any two of `c1`, `c2`, `c3`, a
    /// quorum of one can miss a quorum of the other):
    /// - from `c1` to `c1`, it returns `c1`
    /// - from `c1` to `c2`, it returns `c1c2`
    /// - from `c1c2` to `c2`, it returns `c2`
    /// - from `c1c2` to `c1`, it returns `c1`
    /// - from `c1c2` to `c3`, it returns `c1c2c3`
    ///
    /// * `changes`: The changes to apply to the current membership configuration.
    /// * `retain` specifies whether to retain the removed voters as learners, i.e., nodes that
    ///   continue to receive log replication from the leader.
    /// * `allow_unproven` is `Config::allow_unproven_read_write_intersection`. It lets the new
    ///   configuration pass when OpenRaft cannot prove that its read quorums intersect its write
    ///   quorums.
    ///
    /// A Result containing the new membership configuration and whether it is the joint one, if
    /// the operation succeeds, or a `ChangeMembershipError` if an error occurs.
    ///
    /// This function ensures that the cluster will have at least one voter in the new membership
    /// configuration.
    pub(crate) fn next_membership<Edit, E>(
        &self,
        change: Edit,
        retain: bool,
        allow_unproven: bool,
    ) -> Result<(Mem, bool), ChangeMembershipError<CLID, Mem::NodeId, E>>
    where
        Edit: RaftMembershipEdit<Mem>,
        E: ErrorSource,
    {
        self.ensure_committed()?;

        let effective = self.effective().membership();
        let wrap = |error| E::from_error(&error);
        let goal = change.apply(effective);
        let mut goal = goal.map_err(|error| error.map_user_defined(wrap))?;
        goal.ensure_non_empty_config()?;
        goal.ensure_voters_have_nodes().map_err(MembershipError::<_, E>::from)?;
        goal.ensure_read_write_quorums_intersect(allow_unproven)?;
        if !retain {
            goal = effective.drop_voters_demoted_to_learners(goal);
        }

        if effective.supports_direct_transition_to(&goal) {
            return Ok((goal, false));
        }

        let joint = effective.joint_with(&goal);
        Ok((joint, true))
    }

    /// Validates a membership a caller wants appended as one log entry, without an intermediate
    /// joint membership.
    ///
    /// It rejects the request unless every one of these holds:
    ///
    /// - The last membership log is committed, so the effective membership is also the last
    ///   committed one, the parent this transition starts from.
    /// - `proposed` is a valid membership on its own, and it defines a quorum. A public constructor
    ///   is not enough of a guarantee: deserialization, [`Membership::new()`],
    ///   [`Membership::default()`] and [`Membership::new_with_defaults()`] can all produce a value
    ///   with no voter set at all, or with an empty one.
    /// - `intersects_with()` proves that every read quorum of `proposed` intersects every write
    ///   quorum. With `allow_unproven`, which is `Config::allow_unproven_read_write_intersection`,
    ///   it may also return `None`.
    /// - `proposed` keeps the [`Node`] of every node id the effective membership already knows.
    /// - The transition from the effective membership to `proposed` is one that a direct append
    ///   supports.
    ///
    /// [`Membership::new()`]: crate::Membership::new
    /// [`Membership::default()`]: crate::Membership::default
    /// [`Membership::new_with_defaults()`]: crate::Membership::new_with_defaults
    /// [`Node`]: crate::Node
    pub(crate) fn validate_append_membership<E>(
        &self,
        proposed: &Mem,
        allow_unproven: bool,
    ) -> Result<(), ChangeMembershipError<CLID, Mem::NodeId, E>>
    where
        E: ErrorSource,
    {
        self.ensure_committed()?;
        proposed.ensure_non_empty_config()?;
        proposed.ensure_voters_have_nodes().map_err(MembershipError::<_, E>::from)?;
        proposed.ensure_read_write_quorums_intersect(allow_unproven)?;

        let effective = self.effective().membership();

        effective.ensure_node_metadata_unchanged(proposed)?;

        let compatible = effective.supports_direct_transition_to(proposed);
        if !compatible {
            let err = UnsupportedMembershipTransition {
                previous: effective.to_string(),
                proposed: proposed.to_string(),
            };
            return Err(err.into());
        }

        Ok(())
    }

    /// Ensures that the latest membership has been committed.
    ///
    /// Returns Ok if the last membership is committed, or an InProgress error
    /// otherwise, to indicate a change-membership request should be rejected.
    fn ensure_committed(&self) -> Result<(), InProgress<CLID>> {
        let effective = self.effective();
        let committed = self.committed();

        if effective.log_id() == committed.log_id() {
            return Ok(());
        }

        Err(InProgress {
            committed: committed.log_id().clone(),
            membership_log_id: effective.log_id().clone(),
        })
    }

    /// Commit the effective membership config if `committed_log_id` is greater than or equal to
    /// its log id.
    ///
    /// Committing replaces `self.committed`(the membership state machine) with
    /// `self.effective`(the last membership log).
    pub(crate) fn commit(&mut self, committed_log_id: &Option<LogId<CLID>>) {
        let current = self.committed.log_id().clone();
        let last = self.effective().log_id().clone();

        // `current == last` means the effective membership is already committed.
        if committed_log_id >= &last && current < last {
            debug_assert!(committed_log_id.index() >= last.index());
            self.committed = self.effective.clone();
        }
    }

    /// Install the membership config carried by a snapshot.
    ///
    /// A snapshot stores the last membership applied to its state machine, so this membership is
    /// committed. It updates `self.committed`(the membership state machine), and may also update
    /// `self.effective`(the last membership log), each only when the snapshot is newer than the
    /// corresponding local membership.
    pub(crate) fn install_membership_snapshot(
        &mut self,
        membership_snapshot: Arc<StoredMembership<CLID, Mem>>,
        snapshot_last_log_index: u64,
    ) {
        // Snapshot install purges every log entry up to snapshot_last_log_index.
        let effective_is_purged = self.effective.log_id().index() <= Some(snapshot_last_log_index);

        if effective_is_purged {
            self.committed = membership_snapshot.clone();
            self.effective = membership_snapshot;
            return;
        }

        if membership_snapshot.log_id().index() >= self.effective.log_id().index() {
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
    pub(crate) fn append(&mut self, m: Arc<StoredMembership<CLID, Mem>>) {
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
    pub(crate) fn truncate(&mut self, since: u64) -> Option<Arc<StoredMembership<CLID, Mem>>> {
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
    pub(crate) fn set_effective(&mut self, e: Arc<StoredMembership<CLID, Mem>>) {
        self.effective = e
    }

    /// Returns a reference to the last committed membership config.
    ///
    /// A committed membership config may or may not be the same as the effective one.
    pub fn committed(&self) -> &Arc<StoredMembership<CLID, Mem>> {
        &self.committed
    }

    /// Returns a reference to the presently effective membership config.
    ///
    /// In openraft the last seen membership config, whether committed or not, is the effective
    /// one.
    ///
    /// A committed membership config may or may not be the same as the effective one.
    pub fn effective(&self) -> &Arc<StoredMembership<CLID, Mem>> {
        &self.effective
    }
}

impl<CLID, Mem> Validate for MembershipState<CLID, Mem>
where
    CLID: RaftCommittedLeaderId,
    Mem: RaftMembership,
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        validit::less_equal!(self.committed.log_id(), self.effective.log_id());
        validit::less_equal!(self.committed.log_id().index(), self.effective.log_id().index());
        Ok(())
    }
}
