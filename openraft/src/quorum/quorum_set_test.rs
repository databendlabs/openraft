use maplit::btreeset;

use crate::quorum::AsJoint;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;

#[test]
fn test_simple_quorum_set_impl() -> anyhow::Result<()> {
    // Vec as majority quorum set
    {
        let m12345 = vec![1, 2, 3, 4, 5];

        assert!(!m12345.is_quorum([0].iter()));
        assert!(!m12345.is_quorum([0, 1, 2].iter()));
        assert!(!m12345.is_quorum([6, 7, 8].iter()));
        assert!(m12345.is_quorum([1, 2, 3].iter()));
        assert!(m12345.is_quorum([3, 4, 5].iter()));
        assert!(m12345.is_quorum([1, 3, 4, 5].iter()));
    }

    // BTreeSet as majority quorum set
    {
        let m12345 = btreeset! {1,2,3,4,5};

        assert!(!m12345.is_quorum([0].iter()));
        assert!(!m12345.is_quorum([0, 1, 2].iter()));
        assert!(!m12345.is_quorum([6, 7, 8].iter()));
        assert!(m12345.is_quorum([1, 2, 3].iter()));
        assert!(m12345.is_quorum([3, 4, 5].iter()));
        assert!(m12345.is_quorum([1, 3, 4, 5].iter()));
    }

    Ok(())
}

#[test]
fn test_joint_quorum_set_impl() -> anyhow::Result<()> {
    // Vec<BTreeSet> as majority quorum set
    {
        let m12345 = vec![btreeset! {1,2,3,4,5}];
        let qs = m12345.as_joint();

        assert!(!qs.is_quorum([0].iter()));
        assert!(!qs.is_quorum([0, 1, 2].iter()));
        assert!(!qs.is_quorum([6, 7, 8].iter()));
        assert!(qs.is_quorum([1, 2, 3].iter()));
        assert!(qs.is_quorum([3, 4, 5].iter()));
        assert!(qs.is_quorum([1, 3, 4, 5].iter()));
    }

    // Vec<BTreeSet, BTreeSet> as joint-of-majority quorum set
    {
        let m12345_678 = vec![btreeset! {1,2,3,4,5}, btreeset! {6,7,8}];
        let qs = m12345_678.as_joint();

        assert!(!qs.is_quorum([0].iter()));
        assert!(!qs.is_quorum([0, 1, 2].iter()));
        assert!(!qs.is_quorum([6, 7, 8].iter()));
        assert!(!qs.is_quorum([1, 2, 3].iter()));
        assert!(qs.is_quorum([1, 2, 3, 6, 7].iter()));
        assert!(qs.is_quorum([1, 2, 3, 4, 7, 8].iter()));
    }

    // Vec<&[], &[]> as joint-of-majority quorum set
    {
        let m12345_678: Vec<&[usize]> = vec![&[1, 2, 3, 4, 5], &[6, 7, 8]];
        let qs = m12345_678.as_joint();

        assert!(!qs.is_quorum([0].iter()));
        assert!(!qs.is_quorum([0, 1, 2].iter()));
        assert!(!qs.is_quorum([6, 7, 8].iter()));
        assert!(!qs.is_quorum([1, 2, 3].iter()));
        assert!(qs.is_quorum([1, 2, 3, 6, 7].iter()));
        assert!(qs.is_quorum([1, 2, 3, 4, 7, 8].iter()));
    }

    // Vec<Vec, Vec> as joint-of-majority quorum set
    {
        let m12345_678 = vec![vec![1, 2, 3, 4, 5], vec![6, 7, 8]];
        let qs = m12345_678.as_joint();

        assert!(!qs.is_quorum([0].iter()));
        assert!(!qs.is_quorum([0, 1, 2].iter()));
        assert!(!qs.is_quorum([6, 7, 8].iter()));
        assert!(!qs.is_quorum([1, 2, 3].iter()));
        assert!(qs.is_quorum([1, 2, 3, 6, 7].iter()));
        assert!(qs.is_quorum([1, 2, 3, 4, 7, 8].iter()));
    }

    // Vec<Vec, Vec> into Joint
    {
        let m12345_678 = vec![vec![1, 2, 3, 4, 5], vec![6, 7, 8]];
        let qs = Joint::from(m12345_678);

        assert!(!qs.is_quorum([0].iter()));
        assert!(!qs.is_quorum([0, 1, 2].iter()));
        assert!(!qs.is_quorum([6, 7, 8].iter()));
        assert!(!qs.is_quorum([1, 2, 3].iter()));
        assert!(qs.is_quorum([1, 2, 3, 6, 7].iter()));
        assert!(qs.is_quorum([1, 2, 3, 4, 7, 8].iter()));
    }
    Ok(())
}

#[test]
fn test_ids() -> anyhow::Result<()> {
    {
        let m12345: &[u64] = &[1, 2, 3, 4, 5];
        assert_eq!(btreeset! {1,2,3,4,5}, m12345.ids().collect());
    }

    {
        let m12345 = vec![1, 2, 3, 4, 5];
        assert_eq!(btreeset! {1,2,3,4,5}, m12345.ids().collect());
    }

    {
        let m12345 = btreeset! {1,2,3,4,5};
        assert_eq!(btreeset! {1,2,3,4,5}, m12345.ids().collect());
    }

    {
        let m12345_678 = vec![btreeset! {1,2,3,4,5}, btreeset! {4,5,6,7,8}];
        let qs = m12345_678.as_joint();

        assert_eq!(btreeset! {1,2,3,4,5,6,7,8}, qs.ids().collect());
    }

    Ok(())
}
