use std::sync::Arc;

use maplit::btreeset;

use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;

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
fn test_membership_state_is_member() -> anyhow::Result<()> {
    let x = MembershipState::<UTConfig>::new(effmem(1, 1, m1()), effmem(3, 4, m123_345()));

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
        MembershipState::<UTConfig>::new(
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 2)), m1())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 1, 4)), m123_345())),
        )
    };

    // Smaller new committed wont take effect.
    {
        let mut x = new();
        let res = x.update_committed(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m12())));
        assert!(res.is_none());
        assert_eq!(&Some(log_id(2, 1, 2)), x.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), x.effective().log_id());
    }

    // Update committed, not effective.
    {
        let mut x = new();
        let res = x.update_committed(Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m12())));
        assert!(res.is_none());
        assert_eq!(&Some(log_id(2, 1, 3)), x.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), x.effective().log_id());
    }

    // Update both
    {
        let mut x = new();
        let res = x.update_committed(Arc::new(EffectiveMembership::new(Some(log_id(3, 1, 4)), m12())));
        assert_eq!(Some(x.effective().clone()), res);
        assert_eq!(&Some(log_id(3, 1, 4)), x.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), x.effective().log_id());
        assert_eq!(&m12(), x.effective().membership());
    }

    // Update both, greater log_id.index should update the effective.
    // Because leader may have a smaller log_id that is committed.
    {
        let mut x = new();
        let res = x.update_committed(Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 5)), m12())));
        assert_eq!(Some(x.effective().clone()), res);
        assert_eq!(&Some(log_id(2, 1, 5)), x.committed().log_id());
        assert_eq!(&Some(log_id(2, 1, 5)), x.effective().log_id());
        assert_eq!(&m12(), x.effective().membership());
    }

    Ok(())
}

#[test]
fn test_membership_state_append() -> anyhow::Result<()> {
    let new = || MembershipState::<UTConfig>::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));

    let mut ms = new();
    ms.append(effmem(4, 5, m12()));

    assert_eq!(&Some(log_id(3, 1, 4)), ms.committed().log_id());
    assert_eq!(&Some(log_id(4, 1, 5)), ms.effective().log_id());
    assert_eq!(&m12(), ms.effective().membership());

    Ok(())
}

#[test]
fn test_membership_state_commit() -> anyhow::Result<()> {
    let new = || MembershipState::<UTConfig>::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));

    // Less than committed
    {
        let mut ms = new();
        ms.commit(&Some(log_id(1, 1, 1)));
        assert_eq!(&Some(log_id(2, 1, 2)), ms.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), ms.effective().log_id());
    }

    // Equal committed
    {
        let mut ms = new();
        ms.commit(&Some(log_id(2, 1, 2)));
        assert_eq!(&Some(log_id(2, 1, 2)), ms.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), ms.effective().log_id());
    }

    // Greater than committed, smaller than effective
    {
        let mut ms = new();
        ms.commit(&Some(log_id(2, 1, 3)));
        assert_eq!(&Some(log_id(2, 1, 2)), ms.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), ms.effective().log_id());
    }

    // Greater than committed, equal effective
    {
        let mut ms = new();
        ms.commit(&Some(log_id(3, 1, 4)));
        assert_eq!(&Some(log_id(3, 1, 4)), ms.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), ms.effective().log_id());
    }

    Ok(())
}

#[test]
fn test_membership_state_truncate() -> anyhow::Result<()> {
    let new = || MembershipState::<UTConfig>::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));

    {
        let mut ms = new();
        let res = ms.truncate(5);
        assert!(res.is_none());
        assert_eq!(&Some(log_id(2, 1, 2)), ms.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), ms.effective().log_id());
    }

    {
        let mut ms = new();
        let res = ms.truncate(4);
        assert_eq!(&Some(log_id(2, 1, 2)), res.unwrap().log_id());
        assert_eq!(&Some(log_id(2, 1, 2)), ms.committed().log_id());
        assert_eq!(&Some(log_id(2, 1, 2)), ms.effective().log_id());
    }

    {
        let mut ms = new();
        let res = ms.truncate(3);
        assert_eq!(&Some(log_id(2, 1, 2)), res.unwrap().log_id());
        assert_eq!(&Some(log_id(2, 1, 2)), ms.committed().log_id());
        assert_eq!(&Some(log_id(2, 1, 2)), ms.effective().log_id());
    }

    Ok(())
}
