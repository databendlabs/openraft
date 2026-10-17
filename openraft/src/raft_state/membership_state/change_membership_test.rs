use std::sync::Arc;

use maplit::btreeset;

use crate::ChangeMembers;
use crate::Membership;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::errors::ChangeMembershipError;
use crate::errors::EmptyMembership;
use crate::errors::InProgress;
use crate::errors::LeaderNotInJointMembership;
use crate::errors::LearnerNotFound;
use crate::type_config::alias::MembershipStateOf;
use crate::type_config::alias::StoredMembershipOf;

/// Create an Arc<StoredMembership>
fn effmem(term: u64, index: u64, m: Membership<u64, ()>) -> Arc<StoredMembershipOf<UTConfig>> {
    let lid = Some(log_id(term, 1, index));
    Arc::new(StoredMembershipOf::<UTConfig>::new(lid, m))
}

fn m1() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1}], [])
}

fn m12() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1,2}], [])
}

fn m123_345() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {3,4,5}], [])
}

#[test]
fn test_next_membership_not_committed() -> anyhow::Result<()> {
    let new = || MembershipStateOf::<UTConfig>::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));
    let res = new().next_membership(&1, ChangeMembers::AddVoterIds(btreeset! {1}), false);

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
fn test_next_membership_empty_voters() -> anyhow::Result<()> {
    let new = || MembershipStateOf::<UTConfig>::new(effmem(3, 4, m1()), effmem(3, 4, m1()));
    let res = new().next_membership(&1, ChangeMembers::RemoveVoters(btreeset! {1}), false);

    assert_eq!(Err(ChangeMembershipError::EmptyMembership(EmptyMembership {})), res);

    Ok(())
}

#[test]
fn test_next_membership_learner_not_found() -> anyhow::Result<()> {
    let new = || MembershipStateOf::<UTConfig>::new(effmem(3, 4, m1()), effmem(3, 4, m1()));
    let res = new().next_membership(&1, ChangeMembers::AddVoterIds(btreeset! {2}), false);

    assert_eq!(
        Err(ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: 2 })),
        res
    );

    Ok(())
}

#[test]
fn test_next_membership_retain_learner() -> anyhow::Result<()> {
    let new = || MembershipStateOf::<UTConfig>::new(effmem(3, 4, m12()), effmem(3, 4, m123_345()));

    // Do not leave removed voters as learner
    let res = new().next_membership(&1, ChangeMembers::RemoveVoters(btreeset! {1,2}), false);
    assert_eq!(
        Ok(Membership::new_with_defaults(vec![btreeset! {3,4,5}], [3, 4, 5])),
        res
    );

    // Leave removed voters as learner
    let res = new().next_membership(&1, ChangeMembers::RemoveVoters(btreeset! {1,2}), true);
    assert_eq!(
        Ok(Membership::new_with_defaults(vec![btreeset! {3,4,5}], [1, 2, 3, 4, 5])),
        res
    );

    Ok(())
}

/// From `[{1,2,3}, {3,4,5}]`, removing voter 3 bridges through `[{3,4,5}, {4,5}]`. Leader 3 is in
/// that joint membership, and leader 1 only if the removed voters are retained as learners.
#[test]
fn test_next_membership_joint_leaves_out_leader() -> anyhow::Result<()> {
    let new = || MembershipStateOf::<UTConfig>::new(effmem(3, 4, m123_345()), effmem(3, 4, m123_345()));
    let joint = vec![btreeset! {3,4,5}, btreeset! {4,5}];

    let res = new().next_membership(&3, ChangeMembers::RemoveVoters(btreeset! {3}), false);
    assert_eq!(Ok(Membership::new_with_defaults(joint.clone(), [])), res);

    let res = new().next_membership(&1, ChangeMembers::RemoveVoters(btreeset! {3}), false);
    assert_eq!(
        Err(ChangeMembershipError::LeaderNotInJointMembership(
            LeaderNotInJointMembership {
                leader_id: 1,
                joint: joint.clone(),
            }
        )),
        res
    );

    let res = new().next_membership(&1, ChangeMembers::RemoveVoters(btreeset! {3}), true);
    assert_eq!(Ok(Membership::new_with_defaults(joint, [1, 2])), res);

    Ok(())
}
