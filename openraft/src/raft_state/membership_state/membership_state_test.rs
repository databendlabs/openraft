use std::sync::Arc;

use maplit::btreeset;

use crate::testing::log_id;
use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;

/// Create an Arc<EffectiveMembership>
fn effmem(term: u64, index: u64, m: Membership<u64, ()>) -> Arc<EffectiveMembership<u64, ()>> {
    let lid = Some(log_id(term, 1, index));
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
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 2)), m1())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 1, 4)), m123_345())),
        )
    };

    // Smaller new committed wont take effect.
    {
        let mut x = new();
        let res = x.update_committed(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m12())), 1);
        assert!(res.is_none());
        assert_eq!(&Some(log_id(2, 1, 2)), x.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), x.effective().log_id());
    }

    // Update committed, not effective.
    {
        let mut x = new();
        let res = x.update_committed(Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m12())), 3);
        assert!(res.is_none());
        assert_eq!(&Some(log_id(2, 1, 3)), x.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), x.effective().log_id());
    }

    // Update both
    {
        let mut x = new();
        let res = x.update_committed(Arc::new(EffectiveMembership::new(Some(log_id(3, 1, 4)), m12())), 4);
        assert_eq!(Some(x.effective().clone()), res);
        assert_eq!(&Some(log_id(3, 1, 4)), x.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), x.effective().log_id());
        assert_eq!(&m12(), x.effective().membership());
    }

    // Update both, greater log_id.index should update the effective.
    // Because leader may have a smaller log_id that is committed.
    {
        let mut x = new();
        let res = x.update_committed(Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 5)), m12())), 5);
        assert_eq!(Some(x.effective().clone()), res);
        assert_eq!(&Some(log_id(2, 1, 5)), x.committed().log_id());
        assert_eq!(&Some(log_id(2, 1, 5)), x.effective().log_id());
        assert_eq!(&m12(), x.effective().membership());
    }

    Ok(())
}

/// The snapshot purges the log that backs the local effective membership, so the effective
/// membership must be replaced even though the snapshot's membership has a smaller log index.
#[test]
fn test_update_committed_resets_purged_effective() -> anyhow::Result<()> {
    let snapshot_membership = effmem(5, 4, m12());
    let mut ms = MembershipState::new(effmem(2, 2, m1()), effmem(4, 8, m123_345()));

    let res = ms.update_committed(snapshot_membership.clone(), 9);

    assert_eq!(Some(snapshot_membership.clone()), res);
    assert_eq!(
        MembershipState::new(snapshot_membership.clone(), snapshot_membership),
        ms
    );

    Ok(())
}

/// The purge boundary is inclusive: an effective membership at exactly the snapshot's last log
/// index is purged too.
#[test]
fn test_update_committed_resets_effective_at_purge_boundary() -> anyhow::Result<()> {
    let snapshot_membership = effmem(5, 4, m12());
    let mut ms = MembershipState::new(effmem(2, 2, m1()), effmem(4, 8, m123_345()));

    let res = ms.update_committed(snapshot_membership.clone(), 8);

    assert_eq!(Some(snapshot_membership.clone()), res);
    assert_eq!(
        MembershipState::new(snapshot_membership.clone(), snapshot_membership),
        ms
    );

    Ok(())
}

/// An effective membership the snapshot does not cover still backed by a log entry, thus it is
/// kept.
#[test]
fn test_update_committed_keeps_uncovered_effective() -> anyhow::Result<()> {
    let snapshot_membership = effmem(2, 4, m12());
    let local_effective = effmem(4, 8, m123_345());
    let mut ms = MembershipState::new(snapshot_membership.clone(), local_effective.clone());

    let res = ms.update_committed(snapshot_membership.clone(), 7);

    assert!(res.is_none());
    assert_eq!(MembershipState::new(snapshot_membership, local_effective), ms);

    Ok(())
}

#[test]
fn test_membership_state_append() -> anyhow::Result<()> {
    let new = || MembershipState::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));

    let mut ms = new();
    ms.append(effmem(4, 5, m12()));

    assert_eq!(&Some(log_id(3, 1, 4)), ms.committed().log_id());
    assert_eq!(&Some(log_id(4, 1, 5)), ms.effective().log_id());
    assert_eq!(&m12(), ms.effective().membership());

    Ok(())
}

#[test]
fn test_membership_state_commit() -> anyhow::Result<()> {
    let new = || MembershipState::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));

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
    let new = || MembershipState::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));

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
