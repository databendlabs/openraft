use maplit::btreeset;

use crate::EffectiveMembership;
use crate::Membership;
use crate::engine::testing::UTConfig;
use crate::quorum::QuorumSet;

#[test]
fn test_effective_membership_majority() -> anyhow::Result<()> {
    {
        let m12345 = Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3,4,5 }], []);
        let m = EffectiveMembership::<UTConfig>::new(None, m12345);

        assert!(!m.is_quorum([0].iter()));
        assert!(!m.is_quorum([0, 1, 2].iter()));
        assert!(!m.is_quorum([6, 7, 8].iter()));
        assert!(m.is_quorum([1, 2, 3].iter()));
        assert!(m.is_quorum([3, 4, 5].iter()));
        assert!(m.is_quorum([1, 3, 4, 5].iter()));
    }

    {
        let m12345_678 = Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3,4,5 }, btreeset! {6,7,8}], []);
        let m = EffectiveMembership::<UTConfig>::new(None, m12345_678);

        assert!(!m.is_quorum([0].iter()));
        assert!(!m.is_quorum([0, 1, 2].iter()));
        assert!(!m.is_quorum([6, 7, 8].iter()));
        assert!(!m.is_quorum([1, 2, 3].iter()));
        assert!(m.is_quorum([1, 2, 3, 6, 7].iter()));
        assert!(m.is_quorum([1, 2, 3, 4, 7, 8].iter()));
    }

    Ok(())
}
