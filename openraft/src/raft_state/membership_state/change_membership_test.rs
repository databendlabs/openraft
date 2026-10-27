use std::sync::Arc;

use maplit::btreemap;
use maplit::btreeset;

use crate::Membership;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::errors::ChangeMembershipError;
use crate::errors::EmptyMembership;
use crate::errors::InProgress;
use crate::errors::LearnerNotFound;
use crate::impls::ChangeMembers;
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
    let res = new().next_membership::<_, anyerror::AnyError>(ChangeMembers::AddVoterIds(btreeset! {1}), false, false);

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
    let res = new().next_membership::<_, anyerror::AnyError>(ChangeMembers::RemoveVoters(btreeset! {1}), false, false);

    assert_eq!(Err(ChangeMembershipError::EmptyMembership(EmptyMembership {})), res);

    Ok(())
}

#[test]
fn test_next_membership_learner_not_found() -> anyhow::Result<()> {
    let new = || MembershipStateOf::<UTConfig>::new(effmem(3, 4, m1()), effmem(3, 4, m1()));
    let res = new().next_membership::<_, anyerror::AnyError>(ChangeMembers::AddVoterIds(btreeset! {2}), false, false);

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
    let res =
        new().next_membership::<_, anyerror::AnyError>(ChangeMembers::RemoveVoters(btreeset! {1,2}), false, false);
    let expected = Membership::new_with_defaults(vec![btreeset! {3,4,5}], [3, 4, 5]);
    assert_eq!(Ok((expected, false)), res);

    // Leave removed voters as learner
    let res = new().next_membership::<_, anyerror::AnyError>(ChangeMembers::RemoveVoters(btreeset! {1,2}), true, false);
    let expected = Membership::new_with_defaults(vec![btreeset! {3,4,5}], [1, 2, 3, 4, 5]);
    assert_eq!(Ok((expected, false)), res);

    Ok(())
}

/// A change from a joint membership appends a joint membership that keeps every old voter set.
#[test]
fn test_next_membership_from_joint_keeps_old_voter_sets() -> anyhow::Result<()> {
    let state = MembershipStateOf::<UTConfig>::new(effmem(3, 4, m123_345()), effmem(3, 4, m123_345()));
    let joint = vec![btreeset! {1,2,3}, btreeset! {3,4,5}, btreeset! {4}];

    let result =
        state.next_membership::<_, anyerror::AnyError>(ChangeMembers::ReplaceAllVoters(btreeset! {4}), false, false);
    let expected = Membership::new_with_defaults(joint.clone(), []);
    assert_eq!(Ok((expected, true)), result);

    let result =
        state.next_membership::<_, anyerror::AnyError>(ChangeMembers::ReplaceAllVoters(btreeset! {4}), true, false);
    let expected = Membership::new_with_defaults(joint, []);
    assert_eq!(Ok((expected, true)), result);

    Ok(())
}

/// From a committed membership, `next_membership()` returns the target of `ReplaceAllVoters` when
/// the step to it is proven, and the joint of the two otherwise.
#[test]
fn test_next_membership_returns_target_or_joint() -> anyhow::Result<()> {
    let nodes = || vec![1, 2, 3, 4, 5, 6, 7, 8, 9];
    let c1 = || btreeset! {1,2,3};
    let c2 = || btreeset! {3,4,5};
    let c3 = || btreeset! {7,8,9};

    #[allow(clippy::redundant_closure)]
    let new_mem = |voter_ids, ns| Membership::<u64, ()>::new_with_defaults(voter_ids, ns);

    // The result of `ReplaceAllVoters(voter_ids)` when `m` is the committed and effective membership.
    let next_step = |m: &Membership<u64, ()>, voter_ids, retain: bool| {
        let state = MembershipStateOf::<UTConfig>::new(effmem(3, 4, m.clone()), effmem(3, 4, m.clone()));
        state.next_membership::<_, anyerror::AnyError>(ChangeMembers::ReplaceAllVoters(voter_ids), retain, false)
    };

    let m1 = new_mem(vec![c1()], nodes());
    let m12 = new_mem(vec![c1(), c2()], nodes());

    let res = next_step(&m1, c1(), false);
    assert_eq!(Ok((m1.clone(), false)), res);

    let res = next_step(&m1, c2(), false);
    assert_eq!(Ok((m12.clone(), true)), res);

    let res = next_step(&m12, c1(), false);
    let expected = new_mem(vec![c1()], vec![1, 2, 3, 6, 7, 8, 9]);
    assert_eq!(Ok((expected, false)), res);

    let res = next_step(&m12, c2(), false);
    let expected = new_mem(vec![c2()], vec![3, 4, 5, 6, 7, 8, 9]);
    assert_eq!(Ok((expected, false)), res);

    let res = next_step(&m12, c3(), false);
    let expected = new_mem(vec![c1(), c2(), c3()], nodes());
    assert_eq!(Ok((expected, true)), res);

    let res = next_step(&m12, c3(), true);
    let expected = new_mem(vec![c1(), c2(), c3()], nodes());
    assert_eq!(Ok((expected, true)), res);

    // Turn removed members to learners

    let m23_with_learners_old = new_mem(vec![c2(), c3()], vec![1, 2]);
    let res = next_step(&m23_with_learners_old, c3(), true);
    let expected = new_mem(vec![c3()], vec![1, 2, 3, 4, 5]);
    assert_eq!(Ok((expected, false)), res);

    Ok(())
}

/// A batch that removes voter 2 and adds voter 5 takes two steps, because quorum `{1,2}` of
/// `{1,2,3}` and quorum `{3,5}` of `{1,3,5}` are disjoint.
#[test]
fn test_next_membership_batch_takes_two_steps() -> anyhow::Result<()> {
    let rm_2_add_5 = || {
        ChangeMembers::Batch(vec![
            ChangeMembers::RemoveVoters(btreeset! {2}),
            ChangeMembers::AddVoters(btreemap! {5=>()}),
        ])
    };

    let m123 = Membership::new_with_defaults(vec![btreeset! {1,2,3}], []);
    let state = MembershipStateOf::<UTConfig>::new(effmem(3, 4, m123.clone()), effmem(3, 4, m123));
    let result = state.next_membership::<_, anyerror::AnyError>(rm_2_add_5(), false, false);
    let joint = Membership::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {1,3,5}], []);
    assert_eq!(Ok((joint.clone(), true)), result);

    let state = MembershipStateOf::<UTConfig>::new(effmem(3, 5, joint.clone()), effmem(3, 5, joint));
    let result = state.next_membership::<_, anyerror::AnyError>(rm_2_add_5(), false, false);
    let expected = Membership::new_with_defaults(vec![btreeset! {1,3,5}], []);
    assert_eq!(Ok((expected, false)), result);

    Ok(())
}
