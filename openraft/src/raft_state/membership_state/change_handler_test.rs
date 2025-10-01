use std::sync::Arc;

use maplit::btreeset;

use crate::ChangeMembers;
use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::error::ChangeMembershipError;
use crate::error::EmptyMembership;
use crate::error::InProgress;
use crate::error::LearnerNotFound;

/// Create an Arc<EffectiveMembership>
fn effmem(term: u64, index: u64, m: Membership<UTConfig>) -> Arc<EffectiveMembership<UTConfig>> {
    let lid = Some(log_id(term, 1, index));
    Arc::new(EffectiveMembership::new(lid, m))
}

fn m1() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {1}], [])
}

fn m12() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {1,2}], [])
}

fn m123_345() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {3,4,5}], [])
}

#[test]
fn test_apply_not_committed() -> anyhow::Result<()> {
    let new = || MembershipState::<UTConfig>::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));
    let res = new().change_handler().apply(ChangeMembers::AddVoterIds(btreeset! {1}), false);

    assert_eq!(
        Err(ChangeMembershipError::InProgress(InProgress {
            committed: Some(log_id(2, 1, 2)),
            membership_log_id: Some(log_id(3, 1, 4))
        })),
        res
    );

    Ok(())
}

#[test]
fn test_apply_empty_voters() -> anyhow::Result<()> {
    let new = || MembershipState::<UTConfig>::new(effmem(3, 4, m1()), effmem(3, 4, m1()));
    let res = new().change_handler().apply(ChangeMembers::RemoveVoters(btreeset! {1}), false);

    assert_eq!(Err(ChangeMembershipError::EmptyMembership(EmptyMembership {})), res);

    Ok(())
}

#[test]
fn test_apply_learner_not_found() -> anyhow::Result<()> {
    let new = || MembershipState::<UTConfig>::new(effmem(3, 4, m1()), effmem(3, 4, m1()));
    let res = new().change_handler().apply(ChangeMembers::AddVoterIds(btreeset! {2}), false);

    assert_eq!(
        Err(ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: 2 })),
        res
    );

    Ok(())
}

#[test]
fn test_apply_retain_learner() -> anyhow::Result<()> {
    let new = || MembershipState::<UTConfig>::new(effmem(3, 4, m12()), effmem(3, 4, m123_345()));

    // Do not leave removed voters as learner
    let res = new().change_handler().apply(ChangeMembers::RemoveVoters(btreeset! {1,2}), false);
    assert_eq!(
        Ok(Membership::new_with_defaults(vec![btreeset! {3,4,5}], [3, 4, 5])),
        res
    );

    // Leave removed voters as learner
    let res = new().change_handler().apply(ChangeMembers::RemoveVoters(btreeset! {1,2}), true);
    assert_eq!(
        Ok(Membership::new_with_defaults(vec![btreeset! {3,4,5}], [1, 2, 3, 4, 5])),
        res
    );

    Ok(())
}
