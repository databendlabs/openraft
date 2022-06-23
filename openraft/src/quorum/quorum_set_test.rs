use maplit::btreeset;

use crate::quorum::quorum_set::QuorumSet;

#[test]
fn test_quorum_set_impl() -> anyhow::Result<()> {
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

    // Vec<BTreeSet> as majority quorum set
    {
        let m12345 = vec![btreeset! {1,2,3,4,5}];

        assert!(!m12345.is_quorum([0].iter()));
        assert!(!m12345.is_quorum([0, 1, 2].iter()));
        assert!(!m12345.is_quorum([6, 7, 8].iter()));
        assert!(m12345.is_quorum([1, 2, 3].iter()));
        assert!(m12345.is_quorum([3, 4, 5].iter()));
        assert!(m12345.is_quorum([1, 3, 4, 5].iter()));
    }

    // Vec<BTreeSet, BTreeSet> as joint-of-majority quorum set
    {
        let m12345_678 = vec![btreeset! {1,2,3,4,5}, btreeset! {6,7,8}];

        assert!(!m12345_678.is_quorum([0].iter()));
        assert!(!m12345_678.is_quorum([0, 1, 2].iter()));
        assert!(!m12345_678.is_quorum([6, 7, 8].iter()));
        assert!(!m12345_678.is_quorum([1, 2, 3].iter()));
        assert!(m12345_678.is_quorum([1, 2, 3, 6, 7].iter()));
        assert!(m12345_678.is_quorum([1, 2, 3, 4, 7, 8].iter()));
    }

    // Vec<&[], &[]> as joint-of-majority quorum set
    {
        let m12345_678: Vec<&[usize]> = vec![&[1, 2, 3, 4, 5], &[6, 7, 8]];

        assert!(!m12345_678.is_quorum([0].iter()));
        assert!(!m12345_678.is_quorum([0, 1, 2].iter()));
        assert!(!m12345_678.is_quorum([6, 7, 8].iter()));
        assert!(!m12345_678.is_quorum([1, 2, 3].iter()));
        assert!(m12345_678.is_quorum([1, 2, 3, 6, 7].iter()));
        assert!(m12345_678.is_quorum([1, 2, 3, 4, 7, 8].iter()));
    }
    Ok(())
}
