use std::sync::Arc;

use maplit::btreeset;

use crate::Membership;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
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
fn test_membership_state_is_member() -> anyhow::Result<()> {
    let x = MembershipStateOf::<UTConfig>::new(effmem(1, 1, m1()), effmem(3, 4, m123_345()));

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
        MembershipStateOf::<UTConfig>::new(
            Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(2, 1, 2)), m1())),
            Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(3, 1, 4)), m123_345())),
        )
    };
    let stored = |log_id, membership| Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id), membership));

    // Smaller new committed won't take effect.
    {
        let mut x = new();
        x.install_membership_snapshot(stored(log_id(1, 1, 1), m12()), 1);
        assert_eq!(&Some(log_id(2, 1, 2)), x.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), x.effective().log_id());
    }

    // Update committed, not effective.
    {
        let mut x = new();
        x.install_membership_snapshot(stored(log_id(2, 1, 3), m12()), 3);
        assert_eq!(&Some(log_id(2, 1, 3)), x.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), x.effective().log_id());
    }

    // Update both
    {
        let mut x = new();
        x.install_membership_snapshot(stored(log_id(3, 1, 4), m12()), 4);
        assert_eq!(&Some(log_id(3, 1, 4)), x.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), x.effective().log_id());
        assert_eq!(&m12(), x.effective().membership());
    }

    // Update both, greater log_id.index should update the effective.
    // Because leader may have a smaller log_id that is committed.
    {
        let mut x = new();
        x.install_membership_snapshot(stored(log_id(2, 1, 5), m12()), 5);
        assert_eq!(&Some(log_id(2, 1, 5)), x.committed().log_id());
        assert_eq!(&Some(log_id(2, 1, 5)), x.effective().log_id());
        assert_eq!(&m12(), x.effective().membership());
    }

    Ok(())
}

#[test]
fn test_install_membership_snapshot_resets_purged_effective() -> anyhow::Result<()> {
    let snapshot_membership = effmem(5, 4, m12());
    let mut ms = MembershipStateOf::<UTConfig>::new(effmem(2, 2, m1()), effmem(4, 8, m123_345()));

    ms.install_membership_snapshot(snapshot_membership.clone(), 9);

    assert_eq!(
        MembershipStateOf::<UTConfig>::new(snapshot_membership.clone(), snapshot_membership),
        ms
    );

    Ok(())
}

#[test]
fn test_install_membership_snapshot_resets_effective_at_purge_boundary() -> anyhow::Result<()> {
    let snapshot_membership = effmem(5, 4, m12());
    let mut ms = MembershipStateOf::<UTConfig>::new(effmem(2, 2, m1()), effmem(4, 8, m123_345()));

    ms.install_membership_snapshot(snapshot_membership.clone(), 8);

    assert_eq!(
        MembershipStateOf::<UTConfig>::new(snapshot_membership.clone(), snapshot_membership),
        ms
    );

    Ok(())
}

#[test]
fn test_install_membership_snapshot_keeps_uncovered_effective() -> anyhow::Result<()> {
    let snapshot_membership = effmem(2, 4, m12());
    let local_effective = effmem(4, 8, m123_345());
    let mut ms = MembershipStateOf::<UTConfig>::new(snapshot_membership.clone(), local_effective.clone());

    ms.install_membership_snapshot(snapshot_membership.clone(), 7);

    assert_eq!(
        MembershipStateOf::<UTConfig>::new(snapshot_membership, local_effective),
        ms
    );

    Ok(())
}

#[test]
fn test_membership_state_append() -> anyhow::Result<()> {
    let new = || MembershipStateOf::<UTConfig>::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));

    let mut ms = new();
    ms.append(effmem(4, 5, m12()));

    assert_eq!(&Some(log_id(3, 1, 4)), ms.committed().log_id());
    assert_eq!(&Some(log_id(4, 1, 5)), ms.effective().log_id());
    assert_eq!(&m12(), ms.effective().membership());

    Ok(())
}

#[test]
fn test_membership_state_commit() -> anyhow::Result<()> {
    let new = || MembershipStateOf::<UTConfig>::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));

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

    // Greater than effective
    {
        let mut ms = new();
        ms.commit(&Some(log_id(3, 1, 5)));
        assert_eq!(&Some(log_id(3, 1, 4)), ms.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), ms.effective().log_id());
    }

    // Commit again: the effective membership is already committed; committed stays unchanged.
    {
        let mut ms = new();
        ms.commit(&Some(log_id(3, 1, 4)));
        ms.commit(&Some(log_id(3, 1, 5)));
        assert_eq!(&Some(log_id(3, 1, 4)), ms.committed().log_id());
        assert_eq!(&Some(log_id(3, 1, 4)), ms.effective().log_id());
    }

    // Initial state: both log ids are None; committed stays None.
    {
        let mut ms = MembershipStateOf::<UTConfig>::default();
        ms.commit(&Some(log_id(1, 1, 1)));
        assert_eq!(&None, ms.committed().log_id());
        assert_eq!(&None, ms.effective().log_id());
    }

    Ok(())
}

#[test]
fn test_membership_state_truncate() -> anyhow::Result<()> {
    let new = || MembershipStateOf::<UTConfig>::new(effmem(2, 2, m1()), effmem(3, 4, m123_345()));

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
