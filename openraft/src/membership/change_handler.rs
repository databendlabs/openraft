use crate::error::ChangeMembershipError;
use crate::error::InProgress;
use crate::ChangeMembers;
use crate::Membership;
use crate::MembershipState;
use crate::Node;
use crate::NodeId;

pub(crate) struct ChangeHandler<'m, NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) state: &'m MembershipState<NID, N>,
}

impl<'m, NID, N> ChangeHandler<'m, NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Builds a new membership configuration by applying changes to the current configuration.
    ///
    /// * `changes`: The changes to apply to the current membership configuration.
    /// * `retain` specifies whether to retain the removed voters as a learners, i.e., nodes that
    ///   continue to receive log replication from the leader.
    ///
    /// A Result containing the new membership configuration if the operation succeeds, or a
    /// `ChangeMembershipError` if an error occurs.
    ///
    /// This function ensures that the cluster will have at least one voter in the new membership
    /// configuration.
    pub(crate) fn apply(
        &self,
        change: ChangeMembers<NID, N>,
        retain: bool,
    ) -> Result<Membership<NID, N>, ChangeMembershipError<NID>> {
        self.ensure_committed()?;

        let new_membership = self.state.effective().membership().clone().change(change, retain)?;
        Ok(new_membership)
    }

    /// Ensures that the latest membership has been committed.
    ///
    /// Returns Ok if the last membership is committed, or an InProgress error
    /// otherwise, to indicate a change-membership request should be rejected.
    pub(crate) fn ensure_committed(&self) -> Result<(), InProgress<NID>> {
        let effective = self.state.effective();
        let committed = self.state.committed();

        if effective.log_id() == committed.log_id() {
            // Ok: last membership(effective) is committed
            Ok(())
        } else {
            Err(InProgress {
                committed: *committed.log_id(),
                membership_log_id: *effective.log_id(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use maplit::btreemap;
    use maplit::btreeset;

    use crate::error::ChangeMembershipError;
    use crate::error::EmptyMembership;
    use crate::error::InProgress;
    use crate::error::LearnerNotFound;
    use crate::testing::log_id;
    use crate::ChangeMembers;
    use crate::EffectiveMembership;
    use crate::Membership;
    use crate::MembershipState;

    /// Create an Arc<EffectiveMembership>
    fn effmem(term: u64, index: u64, m: Membership<u64, ()>) -> Arc<EffectiveMembership<u64, ()>> {
        let lid = Some(log_id(term, index));
        Arc::new(EffectiveMembership::new(lid, m))
    }

    fn m1() -> Membership<u64, ()> {
        Membership::new(vec![btreeset! {1}], None)
    }

    fn m12() -> Membership<u64, ()> {
        Membership::new(vec![btreeset! {1,2}], None)
    }

    fn m123_345() -> Membership<u64, ()> {
        Membership::new(vec![btreeset! {1,2,3}, btreeset! {3,4,5}], None)
    }

    #[test]
    fn test_apply_not_committed() -> anyhow::Result<()> {
        let new = || MembershipState::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));
        let res = new().change_handler().apply(ChangeMembers::AddVoterIds(btreeset! {1}), false);

        assert_eq!(
            Err(ChangeMembershipError::InProgress(InProgress {
                committed: Some(log_id(2, 2)),
                membership_log_id: Some(log_id(3, 4))
            })),
            res
        );

        Ok(())
    }

    #[test]
    fn test_apply_empty_voters() -> anyhow::Result<()> {
        let new = || MembershipState::new(effmem(3, 4, m1()), effmem(3, 4, m1()));
        let res = new().change_handler().apply(ChangeMembers::RemoveVoters(btreeset! {1}), false);

        assert_eq!(Err(ChangeMembershipError::EmptyMembership(EmptyMembership {})), res);

        Ok(())
    }

    #[test]
    fn test_apply_learner_not_found() -> anyhow::Result<()> {
        let new = || MembershipState::new(effmem(3, 4, m1()), effmem(3, 4, m1()));
        let res = new().change_handler().apply(ChangeMembers::AddVoterIds(btreeset! {2}), false);

        assert_eq!(
            Err(ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: 2 })),
            res
        );

        Ok(())
    }

    #[test]
    fn test_apply_retain_learner() -> anyhow::Result<()> {
        let new = || MembershipState::new(effmem(3, 4, m12()), effmem(3, 4, m123_345()));

        // Do not leave removed voters as learner
        let res = new().change_handler().apply(ChangeMembers::RemoveVoters(btreeset! {1,2}), false);
        assert_eq!(
            Ok(Membership::new(vec![btreeset! {3,4,5}], btreemap! {3=>(),4=>(),5=>()})),
            res
        );

        // Leave removed voters as learner
        let res = new().change_handler().apply(ChangeMembers::RemoveVoters(btreeset! {1,2}), true);
        assert_eq!(
            Ok(Membership::new(
                vec![btreeset! {3,4,5}],
                btreemap! {1=>(),2=>(),3=>(),4=>(),5=>()}
            )),
            res
        );

        Ok(())
    }
}
