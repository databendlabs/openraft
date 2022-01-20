use std::collections::BTreeMap;

use maplit::btreemap;
use maplit::btreeset;

use crate::Membership;
use crate::NodeId;

#[test]
fn test_membership() -> anyhow::Result<()> {
    let m1 = Membership::new_multi(vec![btreeset! {1}]);
    let m123 = Membership::new_multi(vec![btreeset! {1,2,3}]);
    let m123_345 = Membership::new_multi(vec![btreeset! {1,2,3}, btreeset! {3,4,5}]);

    assert_eq!(Some(btreeset! {1}), m1.get_ith_config(0).cloned());
    assert_eq!(Some(btreeset! {1,2,3}), m123.get_ith_config(0).cloned());
    assert_eq!(Some(btreeset! {1,2,3}), m123_345.get_ith_config(0).cloned());

    assert_eq!(None, m1.get_ith_config(1).cloned());
    assert_eq!(None, m123.get_ith_config(1).cloned());
    assert_eq!(Some(btreeset! {3,4,5}), m123_345.get_ith_config(1).cloned());

    assert_eq!(vec![1], m1.ith_config(0));
    assert_eq!(vec![1, 2, 3], m123.ith_config(0));
    assert_eq!(vec![1, 2, 3], m123_345.ith_config(0));
    assert_eq!(vec![3, 4, 5], m123_345.ith_config(1));

    assert_eq!(&btreeset! {1}, m1.all_members());
    assert_eq!(&btreeset! {1,2,3}, m123.all_members());
    assert_eq!(&btreeset! {1,2,3,4,5}, m123_345.all_members());

    assert!(!m1.is_member(&0));
    assert!(m1.is_member(&1));
    assert!(m123_345.is_member(&4));
    assert!(!m123_345.is_member(&6));

    assert!(!m123.is_in_joint_consensus());
    assert!(m123_345.is_in_joint_consensus());

    assert_eq!(Membership::new_single(btreeset! {3,4,5}), m123_345.to_final_config());

    Ok(())
}

#[test]
fn test_membership_with_learners() -> anyhow::Result<()> {
    // test multi membership with learners
    {
        let m1_2 = Membership::new_multi_with_learners(vec![btreeset! {1}], btreeset! {2});
        let m1_23 = m1_2.add_learner(&3);

        // test learner and membership
        assert_eq!(&btreeset! {1}, m1_2.all_members());
        assert_eq!(&btreeset! {2}, m1_2.all_learners());
        assert!(m1_2.is_learner(&2));

        assert_eq!(&btreeset! {1}, m1_23.all_members());
        assert_eq!(&btreeset! {2,3}, m1_23.all_learners());
        assert!(m1_23.is_learner(&2));
        assert!(m1_23.is_learner(&3));
    }

    // test single membership with learners
    {
        let s1_2 = Membership::new_single_with_learners(btreeset! {1}, btreeset! {2});
        let s1_23 = s1_2.add_learner(&3);

        // test learner and membership
        assert_eq!(&btreeset! {1}, s1_2.all_members());
        assert_eq!(&btreeset! {2}, s1_2.all_learners());
        assert!(s1_2.is_learner(&2));

        assert_eq!(&btreeset! {1}, s1_23.all_members());
        assert_eq!(&btreeset! {2,3}, s1_23.all_learners());
        assert!(s1_23.is_learner(&2));
        assert!(s1_23.is_learner(&3));
    }
    Ok(())
}

#[test]
fn test_membership_update() -> anyhow::Result<()> {
    // --- replace

    let mut m123 = Membership::new_single(btreeset! {1,2,3});
    m123.replace(vec![btreeset! {2,3}, btreeset! {3,4}]);

    assert_eq!(&btreeset! {2,3,4}, m123.all_members());
    assert_eq!(&vec![btreeset! {2,3}, btreeset! {3,4}], m123.get_configs());

    // --- push

    m123.push(btreeset! {3,5});

    assert_eq!(&btreeset! {2,3,4,5}, m123.all_members());
    assert_eq!(
        &vec![btreeset! {2,3}, btreeset! {3,4}, btreeset! {3,5}],
        m123.get_configs()
    );

    // --- to final

    let got = m123.to_final_config();

    assert_eq!(&btreeset! {3,5}, got.all_members());
    assert_eq!(&vec![btreeset! {3,5}], got.get_configs());

    Ok(())
}

#[test]
fn test_membership_update_with_learners() -> anyhow::Result<()> {
    // test multi membership with learners
    {
        let mut m1_2 = Membership::new_multi_with_learners(vec![btreeset! {1}], btreeset! {2});

        // test replace and push, does not affect learners
        m1_2.replace(vec![btreeset! {3,4}]);
        assert_eq!(&btreeset! {3,4}, m1_2.all_members());
        assert_eq!(&btreeset! {2}, m1_2.all_learners());

        m1_2.push(btreeset! {5});
        assert_eq!(&btreeset! {3,4,5}, m1_2.all_members());
        assert_eq!(&btreeset! {2}, m1_2.all_learners());
    }

    // test single membership with learners
    {
        let mut s1_2 = Membership::new_single_with_learners(btreeset! {1}, btreeset! {2});

        // test replace and push, does not affect learners
        s1_2.replace(vec![btreeset! {3,4}]);
        assert_eq!(&btreeset! {3,4}, s1_2.all_members());
        assert_eq!(&btreeset! {2}, s1_2.all_learners());

        s1_2.push(btreeset! {5});
        assert_eq!(&btreeset! {3,4,5}, s1_2.all_members());
        assert_eq!(&btreeset! {2}, s1_2.all_learners());
    }
    Ok(())
}

#[test]
fn test_membership_majority() -> anyhow::Result<()> {
    {
        let m12345 = Membership::new_single(btreeset! {1,2,3,4,5});
        assert!(!m12345.is_majority(&btreeset! {0}));
        assert!(!m12345.is_majority(&btreeset! {0,1,2}));
        assert!(!m12345.is_majority(&btreeset! {6,7,8}));
        assert!(m12345.is_majority(&btreeset! {1,2,3}));
        assert!(m12345.is_majority(&btreeset! {3,4,5}));
        assert!(m12345.is_majority(&btreeset! {1,3,4,5}));
    }

    {
        let m12345_678 = Membership::new_multi(vec![btreeset! {1,2,3,4,5}, btreeset! {6,7,8}]);
        assert!(!m12345_678.is_majority(&btreeset! {0}));
        assert!(!m12345_678.is_majority(&btreeset! {0,1,2}));
        assert!(!m12345_678.is_majority(&btreeset! {6,7,8}));
        assert!(!m12345_678.is_majority(&btreeset! {1,2,3}));
        assert!(m12345_678.is_majority(&btreeset! {1,2,3,6,7}));
        assert!(m12345_678.is_majority(&btreeset! {1,2,3,4,7,8}));
    }

    Ok(())
}

#[test]
fn test_membership_greatest_majority_value() -> anyhow::Result<()> {
    {
        let m123 = Membership::new_single(btreeset! {1,2,3});
        assert_eq!(None, m123.greatest_majority_value(&BTreeMap::<NodeId, u64>::new()));
        assert_eq!(None, m123.greatest_majority_value(&btreemap! {0=>10}));
        assert_eq!(None, m123.greatest_majority_value(&btreemap! {0=>10,1=>10}));
        assert_eq!(Some(&10), m123.greatest_majority_value(&btreemap! {0=>10,1=>10,2=>20}));
        assert_eq!(
            Some(&20),
            m123.greatest_majority_value(&btreemap! {0=>10,1=>10,2=>20,3=>30})
        );
    }

    {
        let m123_678 = Membership::new_multi(vec![btreeset! {1,2,3}, btreeset! {6,7,8}]);
        assert_eq!(None, m123_678.greatest_majority_value(&btreemap! {0=>10}));
        assert_eq!(None, m123_678.greatest_majority_value(&btreemap! {0=>10,1=>10}));
        assert_eq!(None, m123_678.greatest_majority_value(&btreemap! {0=>10,1=>10,2=>20}));
        assert_eq!(
            None,
            m123_678.greatest_majority_value(&btreemap! {0=>10,1=>10,2=>20,6=>15})
        );
        assert_eq!(
            Some(&10),
            m123_678.greatest_majority_value(&btreemap! {0=>10,1=>10,2=>20,6=>15,7=>20})
        );
        assert_eq!(
            Some(&15),
            m123_678.greatest_majority_value(&btreemap! {0=>10,1=>10,2=>20,3=>20,6=>15,7=>20})
        );
    }

    Ok(())
}

#[test]
fn test_membership_is_safe_to() -> anyhow::Result<()> {
    let set123 = || btreeset! {1,2,3};
    let set345 = || btreeset! {3,4,5};
    let set789 = || btreeset! {7,8,9};

    let m123 = Membership::new_single(set123());
    let m345 = Membership::new_single(set345());
    let m123_345 = Membership::new_multi(vec![set123(), set345()]);
    let m345_789 = Membership::new_multi(vec![set345(), set789()]);

    assert!(m123.is_safe_to(&m123));
    assert!(!m123.is_safe_to(&m345));
    assert!(m123.is_safe_to(&m123_345));
    assert!(!m123.is_safe_to(&m345_789));

    assert!(!m345.is_safe_to(&m123));
    assert!(m345.is_safe_to(&m345));
    assert!(m345.is_safe_to(&m123_345));
    assert!(m345.is_safe_to(&m345_789));

    assert!(m123_345.is_safe_to(&m123));
    assert!(m123_345.is_safe_to(&m345));
    assert!(m123_345.is_safe_to(&m123_345));
    assert!(m123_345.is_safe_to(&m345_789));

    assert!(!m345_789.is_safe_to(&m123));
    assert!(m345_789.is_safe_to(&m345));
    assert!(m345_789.is_safe_to(&m123_345));
    assert!(m345_789.is_safe_to(&m345_789));

    Ok(())
}

#[test]
fn test_membership_next_safe() -> anyhow::Result<()> {
    let c1 = || btreeset! {1,2,3};
    let c2 = || btreeset! {3,4,5};
    let c3 = || btreeset! {7,8,9};

    let m1 = Membership::new_single(c1());
    let m2 = Membership::new_single(c2());
    let m12 = Membership::new_multi(vec![c1(), c2()]);
    let m23 = Membership::new_multi(vec![c2(), c3()]);

    assert_eq!(m1, m1.next_safe(c1()));
    assert_eq!(m12, m1.next_safe(c2()));
    assert_eq!(m1, m12.next_safe(c1()));
    assert_eq!(m2, m12.next_safe(c2()));
    assert_eq!(m23, m12.next_safe(c3()));

    Ok(())
}
