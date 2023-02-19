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
fn test_membership_state_is_member() -> anyhow::Result<()> {
    let x = MembershipState::new(effmem(1, 1, m1()), effmem(3, 4, m123_345()));

    assert!(!x.is_voter(&0));
    assert!(x.is_voter(&1));
    assert!(x.is_voter(&2));
    assert!(x.is_voter(&3));
    assert!(x.is_voter(&4));
    assert!(x.is_voter(&5));
    assert!(!x.is_voter(&6));

    Ok(())
}

#[test]
fn test_membership_state_update_committed() -> anyhow::Result<()> {
    let new = || {
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(2, 2)), m1())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m123_345())),
        )
    };

    // Smaller new committed wont take effect.
    {
        let mut x = new();
        let res = x.update_committed(Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12())));
        assert!(res.is_none());
        assert_eq!(Some(log_id(2, 2)), x.committed().log_id);
        assert_eq!(Some(log_id(3, 4)), x.effective().log_id);
    }

    // Update committed, not effective.
    {
        let mut x = new();
        let res = x.update_committed(Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m12())));
        assert!(res.is_none());
        assert_eq!(Some(log_id(2, 3)), x.committed().log_id);
        assert_eq!(Some(log_id(3, 4)), x.effective().log_id);
    }

    // Update both
    {
        let mut x = new();
        let res = x.update_committed(Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m12())));
        assert_eq!(Some(x.effective().clone()), res);
        assert_eq!(Some(log_id(3, 4)), x.committed().log_id);
        assert_eq!(Some(log_id(3, 4)), x.effective().log_id);
        assert_eq!(m12(), x.effective().membership);
    }

    // Update both, greater log_id.index should update the effective.
    // Because leader may have a smaller log_id that is committed.
    {
        let mut x = new();
        let res = x.update_committed(Arc::new(EffectiveMembership::new(Some(log_id(2, 5)), m12())));
        assert_eq!(Some(x.effective().clone()), res);
        assert_eq!(Some(log_id(2, 5)), x.committed().log_id);
        assert_eq!(Some(log_id(2, 5)), x.effective().log_id);
        assert_eq!(m12(), x.effective().membership);
    }

    Ok(())
}

#[test]
fn test_membership_state_append() -> anyhow::Result<()> {
    let new = || MembershipState::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));

    let mut ms = new();
    ms.append(effmem(4, 5, m12()));

    assert_eq!(Some(log_id(3, 4)), ms.committed().log_id);
    assert_eq!(Some(log_id(4, 5)), ms.effective().log_id);
    assert_eq!(m12(), ms.effective().membership);

    Ok(())
}

#[test]
fn test_membership_state_commit() -> anyhow::Result<()> {
    let new = || MembershipState::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));

    // Less than committed
    {
        let mut ms = new();
        ms.commit(&Some(log_id(1, 1)));
        assert_eq!(Some(log_id(2, 2)), ms.committed().log_id);
        assert_eq!(Some(log_id(3, 4)), ms.effective().log_id);
    }

    // Equal committed
    {
        let mut ms = new();
        ms.commit(&Some(log_id(2, 2)));
        assert_eq!(Some(log_id(2, 2)), ms.committed().log_id);
        assert_eq!(Some(log_id(3, 4)), ms.effective().log_id);
    }

    // Greater than committed, smaller than effective
    {
        let mut ms = new();
        ms.commit(&Some(log_id(2, 3)));
        assert_eq!(Some(log_id(2, 2)), ms.committed().log_id);
        assert_eq!(Some(log_id(3, 4)), ms.effective().log_id);
    }

    // Greater than committed, equal effective
    {
        let mut ms = new();
        ms.commit(&Some(log_id(3, 4)));
        assert_eq!(Some(log_id(3, 4)), ms.committed().log_id);
        assert_eq!(Some(log_id(3, 4)), ms.effective().log_id);
    }

    Ok(())
}

#[test]
fn test_membership_state_truncate() -> anyhow::Result<()> {
    let new = || MembershipState::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));

    {
        let mut ms = new();
        let res = ms.truncate(5);
        assert!(res.is_none());
        assert_eq!(Some(log_id(2, 2)), ms.committed().log_id);
        assert_eq!(Some(log_id(3, 4)), ms.effective().log_id);
    }

    {
        let mut ms = new();
        let res = ms.truncate(4);
        assert_eq!(Some(log_id(2, 2)), res.unwrap().log_id);
        assert_eq!(Some(log_id(2, 2)), ms.committed().log_id);
        assert_eq!(Some(log_id(2, 2)), ms.effective().log_id);
    }

    {
        let mut ms = new();
        let res = ms.truncate(3);
        assert_eq!(Some(log_id(2, 2)), res.unwrap().log_id);
        assert_eq!(Some(log_id(2, 2)), ms.committed().log_id);
        assert_eq!(Some(log_id(2, 2)), ms.effective().log_id);
    }

    Ok(())
}

#[test]
fn test_membership_state_next_membership_not_committed() -> anyhow::Result<()> {
    let new = || MembershipState::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));
    let res = new().create_updated_membership(ChangeMembers::Add(btreeset! {1}), false);

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
fn test_membership_state_create_updated_membership_empty_voters() -> anyhow::Result<()> {
    let new = || MembershipState::new(effmem(3, 4, m1()), effmem(3, 4, m1()));
    let res = new().create_updated_membership(ChangeMembers::Remove(btreeset! {1}), false);

    assert_eq!(Err(ChangeMembershipError::EmptyMembership(EmptyMembership {})), res);

    Ok(())
}

#[test]
fn test_membership_state_create_updated_membership_learner_not_found() -> anyhow::Result<()> {
    let new = || MembershipState::new(effmem(3, 4, m1()), effmem(3, 4, m1()));
    let res = new().create_updated_membership(ChangeMembers::Add(btreeset! {2}), false);

    assert_eq!(
        Err(ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: 2 })),
        res
    );

    Ok(())
}

#[test]
fn test_membership_state_create_updated_membership_removed_to_learner() -> anyhow::Result<()> {
    let new = || MembershipState::new(effmem(3, 4, m12()), effmem(3, 4, m123_345()));

    // Do not leave removed voters as learner
    let res = new().create_updated_membership(ChangeMembers::Remove(btreeset! {1,2}), false);
    assert_eq!(
        Ok(Membership::new(vec![btreeset! {3,4,5}], btreemap! {3=>(),4=>(),5=>()})),
        res
    );

    // Leave removed voters as learner
    let res = new().create_updated_membership(ChangeMembers::Remove(btreeset! {1,2}), true);
    assert_eq!(
        Ok(Membership::new(
            vec![btreeset! {3,4,5}],
            btreemap! {1=>(),2=>(),3=>(),4=>(),5=>()}
        )),
        res
    );

    Ok(())
}
