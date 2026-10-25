use maplit::btreeset;

use crate::Membership;
use crate::StoredMembership;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::quorum::QuorumSet;
use crate::type_config::alias::StoredMembershipOf;

#[test]
fn test_stored_membership_nodes_and_quorums() {
    let configs = vec![btreeset! {1, 2, 3}];
    let membership = Membership::<u64, ()>::new_with_defaults(configs.clone(), [4]);
    let log_id = Some(log_id(1, 1, 1));
    let stored = StoredMembership::new(log_id, membership);
    let nodes = stored.nodes().collect::<Vec<_>>();
    assert_eq!(nodes, vec![(&1, &()), (&2, &()), (&3, &()), (&4, &())]);
    let joint_config = stored.get_joint_config();
    assert_eq!(joint_config, &configs);
    let ids = stored.ids().collect::<Vec<_>>();
    assert_eq!(ids, vec![1, 2, 3]);
    let has_quorum = stored.is_quorum([1, 2].iter());
    assert!(has_quorum);
    let lacks_quorum = stored.is_quorum([1, 4].iter());
    assert!(!lacks_quorum);
}

#[test]
fn test_stored_membership_campaigners() {
    let cases = [
        (Membership::<u64, ()>::default(), vec![]),
        (Membership::new_with_defaults(vec![btreeset! {1}], [2]), vec![1]),
        (
            Membership::new_with_defaults(vec![btreeset! {1,2}, btreeset! {2,3}], [4]),
            vec![1, 2, 3],
        ),
    ];
    for (membership, expected) in cases {
        for log_id in [None, Some(log_id(1, 1, 1))] {
            let stored = StoredMembershipOf::<UTConfig>::new(log_id, membership.clone());
            let campaigners = (0..=5).filter(|id| stored.is_campaigner(id)).collect::<Vec<_>>();
            assert_eq!(expected, campaigners);
        }
    }
}

#[test]
fn test_stored_membership_majority() -> anyhow::Result<()> {
    {
        let m12345 = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,4,5 }], []);
        let m = StoredMembershipOf::<UTConfig>::new(None, m12345);

        assert!(!m.is_quorum([0].iter()));
        assert!(!m.is_quorum([0, 1, 2].iter()));
        assert!(!m.is_quorum([6, 7, 8].iter()));
        assert!(m.is_quorum([1, 2, 3].iter()));
        assert!(m.is_quorum([3, 4, 5].iter()));
        assert!(m.is_quorum([1, 3, 4, 5].iter()));
    }

    {
        let m12345_678 = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,4,5 }, btreeset! {6,7,8}], []);
        let m = StoredMembershipOf::<UTConfig>::new(None, m12345_678);

        assert!(!m.is_quorum([0].iter()));
        assert!(!m.is_quorum([0, 1, 2].iter()));
        assert!(!m.is_quorum([6, 7, 8].iter()));
        assert!(!m.is_quorum([1, 2, 3].iter()));
        assert!(m.is_quorum([1, 2, 3, 6, 7].iter()));
        assert!(m.is_quorum([1, 2, 3, 4, 7, 8].iter()));
    }

    Ok(())
}
