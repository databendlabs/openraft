use crate::ChangeMembers;
use crate::Membership;
use crate::MembershipState;
use crate::RaftTypeConfig;
use crate::error::ChangeMembershipError;
use crate::error::InProgress;

/// This struct handles change-membership requests, validating them and applying the changes if
/// the necessary conditions are met. It operates at the `Engine` and `RaftState` level and
/// serves as the outermost API for a consensus engine.
pub(crate) struct ChangeHandler<'m, C>
where C: RaftTypeConfig
{
    pub(crate) state: &'m MembershipState<C>,
}

impl<C> ChangeHandler<'_, C>
where C: RaftTypeConfig
{
    /// Builds a new membership configuration by applying changes to the current configuration.
    ///
    /// * `changes`: The changes to apply to the current membership configuration.
    /// * `retain` specifies whether to retain the removed voters as learners, i.e., nodes that
    ///   continue to receive log replication from the leader.
    ///
    /// A Result containing the new membership configuration if the operation succeeds, or a
    /// `ChangeMembershipError` if an error occurs.
    ///
    /// This function ensures that the cluster will have at least one voter in the new membership
    /// configuration.
    pub(crate) fn apply(
        &self,
        change: ChangeMembers<C>,
        retain: bool,
    ) -> Result<Membership<C>, ChangeMembershipError<C>> {
        self.ensure_committed()?;

        let new_membership = self.state.effective().membership().clone().change(change, retain)?;
        Ok(new_membership)
    }

    /// Ensures that the latest membership has been committed.
    ///
    /// Returns Ok if the last membership is committed, or an InProgress error
    /// otherwise, to indicate a change-membership request should be rejected.
    pub(crate) fn ensure_committed(&self) -> Result<(), InProgress<C>> {
        let effective = self.state.effective();
        let committed = self.state.committed();

        if effective.log_id() == committed.log_id() {
            // Ok: last membership(effective) is committed
            Ok(())
        } else {
            Err(InProgress {
                committed: committed.log_id().clone(),
                membership_log_id: effective.log_id().clone(),
            })
        }
    }
}
