use std::collections::BTreeMap;
use std::option::Option::None;

use maplit::btreemap;
use maplit::btreeset;

use crate::error::NodeIdNotInNodes;
use crate::testing::DummyConfig as Config;
use crate::Membership;
use crate::MessageSummary;
use crate::Node;
use crate::RaftTypeConfig;

#[test]
fn test_membership_summary() -> anyhow::Result<()> {
    let node = |addr: &str, k: &str| Node {
        addr: addr.to_string(),
        data: btreemap! {k.to_string() => k.to_string()},
    };

    let m = Membership::<Config>::new(vec![btreeset! {1,2}, btreeset! {3}], None);
    assert_eq!("members:[{1,2},{3}],learners:[]", m.summary());

    let m = Membership::<Config>::new(vec![btreeset! {1,2}, btreeset! {3}], Some(btreeset! {4}));
    assert_eq!("members:[{1,2},{3}],learners:[4]", m.summary());

    let m = m.set_nodes(Some(btreemap! {
        1=>node("127.0.0.1", "k1"),
        2=>node("127.0.0.2", "k2"),
        3=>node("127.0.0.3", "k3"),
        4=>node("127.0.0.4", "k4"),

    }))?;
    assert_eq!(
        "members:[{1:{127.0.0.1; k1:k1},2:{127.0.0.2; k2:k2}},{3:{127.0.0.3; k3:k3}}],learners:[4:{127.0.0.4; k4:k4}]",
        m.summary()
    );

    Ok(())
}

#[test]
fn test_membership() -> anyhow::Result<()> {
    let m1 = Membership::<Config>::new(vec![btreeset! {1}], None);
    let m123 = Membership::<Config>::new(vec![btreeset! {1,2,3}], None);
    let m123_345 = Membership::<Config>::new(vec![btreeset! {1,2,3}, btreeset! {3,4,5}], None);

    assert_eq!(Some(btreeset! {1}), m1.get_ith_config(0).cloned());
    assert_eq!(Some(btreeset! {1,2,3}), m123.get_ith_config(0).cloned());
    assert_eq!(Some(btreeset! {1,2,3}), m123_345.get_ith_config(0).cloned());

    assert_eq!(None, m1.get_ith_config(1).cloned());
    assert_eq!(None, m123.get_ith_config(1).cloned());
    assert_eq!(Some(btreeset! {3,4,5}), m123_345.get_ith_config(1).cloned());

    assert_eq!(btreeset! {1}, m1.all_members());
    assert_eq!(btreeset! {1,2,3}, m123.all_members());
    assert_eq!(btreeset! {1,2,3,4,5}, m123_345.all_members());

    assert!(!m1.is_member(&0));
    assert!(m1.is_member(&1));
    assert!(m123_345.is_member(&4));
    assert!(!m123_345.is_member(&6));

    assert!(!m123.is_in_joint_consensus());
    assert!(m123_345.is_in_joint_consensus());

    Ok(())
}

#[test]
fn test_membership_with_learners() -> anyhow::Result<()> {
    // test multi membership with learners
    {
        let m1_2 = Membership::<Config>::new(vec![btreeset! {1}], Some(btreeset! {2}));
        let m1_23 = m1_2.add_learner(3, None)?;

        // test learner and membership
        assert_eq!(btreeset! {1}, m1_2.all_members());
        assert_eq!(&btreeset! {2}, m1_2.all_learners());
        assert!(m1_2.is_learner(&2));

        assert_eq!(btreeset! {1}, m1_23.all_members());
        assert_eq!(&btreeset! {2,3}, m1_23.all_learners());
        assert!(m1_23.is_learner(&2));
        assert!(m1_23.is_learner(&3));

        // Adding a member as learner has no effect:

        let m = m1_23.add_learner(1, None)?;
        assert_eq!(btreeset! {1}, m.all_members());

        // Adding a existent learner has no effect:

        let m = m1_23.add_learner(3, None)?;
        assert_eq!(btreeset! {1}, m.all_members());
        assert_eq!(&btreeset! {2,3}, m.all_learners());
    }

    // overlapping members and learners
    {
        let s1_2 = Membership::<Config>::new(vec![btreeset! {1,2,3}, btreeset! {5,6,7}], Some(btreeset! {3,4,5}));
        let x = s1_2.all_learners();
        assert_eq!(&btreeset! {4}, x);
    }

    Ok(())
}

#[test]
fn test_membership_add_learner() -> anyhow::Result<()> {
    let node = |s: &str| Node {
        addr: s.to_string(),
        data: Default::default(),
    };

    let m_1_2 = Membership::<Config>::new(vec![btreeset! {1}, btreeset! {2}], None)
        .set_nodes(Some(btreemap! {1=>node("1"), 2=>node("2")}))?;

    // Add learner that presents in old cluster has no effect.

    let res = m_1_2.add_learner(1, Some(node("3")))?;
    assert_eq!(m_1_2, res);

    // Success to add a learner

    let m_1_2_3 = m_1_2.add_learner(3, Some(node("3")))?;
    assert_eq!(
        Membership::<Config>::new(vec![btreeset! {1}, btreeset! {2}], Some(btreeset! {3}))
            .set_nodes(Some(btreemap! {1=>node("1"), 2=>node("2"), 3=>node("3")}))?,
        m_1_2_3
    );

    // Illegal to add node id without node info into cluster with node info.
    {
        let res = m_1_2.add_learner(3, None);
        assert_eq!(
            Err(NodeIdNotInNodes {
                node_id: 3,
                node_ids: btreeset! {1,2},
            }),
            res
        );
    }

    // Illegal to add node id with node info into cluster without node info.
    {
        let m_1_2 = Membership::<Config>::new(vec![btreeset! {1}, btreeset! {2}], None);

        let res = m_1_2.add_learner(3, Some(node("3")));
        assert_eq!(
            Err(NodeIdNotInNodes {
                node_id: 1,
                node_ids: btreeset! {3},
            }),
            res
        );
    }

    Ok(())
}

#[test]
fn test_membership_check_ndoe_ids_in_nodes() -> anyhow::Result<()> {
    let node = Node::default;

    let mem = |c1, c2, l| Membership::<Config>::new(vec![btreeset! {c1}, btreeset! {c2}], Some(btreeset! {l}));
    let nodes = Some(btreemap! {1 => node(), 2=>node(), 3=>node()});
    let all = || btreeset! {1,2,3};

    assert_eq!(
        Err(NodeIdNotInNodes {
            node_id: 5,
            node_ids: all()
        }),
        mem(5, 2, 3).check_node_ids_in_nodes(&nodes)
    );

    assert_eq!(
        Err(NodeIdNotInNodes {
            node_id: 6,
            node_ids: all()
        }),
        mem(1, 6, 3).check_node_ids_in_nodes(&nodes)
    );

    assert_eq!(
        Err(NodeIdNotInNodes {
            node_id: 7,
            node_ids: all()
        }),
        mem(1, 2, 7).check_node_ids_in_nodes(&nodes)
    );

    assert_eq!(Ok(()), mem(1, 2, 3).check_node_ids_in_nodes(&nodes));

    Ok(())
}

#[test]
fn test_membership_extend_nodes() -> anyhow::Result<()> {
    let node = |s: &str| Node {
        addr: s.to_string(),
        data: Default::default(),
    };

    let ext = |a, b| Membership::<Config>::extend_nodes(a, &b);

    assert_eq!(None, ext(None, None));
    assert_eq!(
        Some(btreemap! {1=>node("1")}),
        ext(None, Some(btreemap! {1=>node("1")}))
    );

    assert_eq!(
        Some(btreemap! {1=>node("1")}),
        ext(Some(btreemap! {1=>node("1")}), None)
    );

    assert_eq!(
        Some(btreemap! {1=>node("1")}),
        ext(Some(btreemap! {1=>node("1")}), Some(btreemap! {1=>node("2")})),
        "existent node will not change"
    );

    assert_eq!(
        Some(btreemap! {1=>node("1"), 2=>node("2")}),
        ext(
            Some(btreemap! {1=>node("1")}),
            Some(btreemap! {1=>node("2"), 2=>node("2")})
        ),
    );

    Ok(())
}

#[test]
fn test_membership_remove_unused_nodes() -> anyhow::Result<()> {
    let node = Node::default;

    let m = Membership::<Config>::new(vec![btreeset! {1,2}, btreeset! {3}], Some(btreeset! {4}));

    assert_eq!(None, m.remove_unused_nodes(&None));

    assert_eq!(
        Some(btreemap! {1=>node(), 2=>node(), 3=>node(), 4=>node(),}),
        m.remove_unused_nodes(&Some(btreemap! {1=>node(), 2=>node(), 3=>node(),4=>node()}))
    );
    assert_eq!(
        Some(btreemap! {1=>node(), 2=>node(), 3=>node(), 4=>node(),}),
        m.remove_unused_nodes(&Some(btreemap! {1=>node(), 2=>node(), 3=>node(), 4=>node(),5=>node()}))
    );

    Ok(())
}

#[test]
fn test_membership_set_nodes() -> anyhow::Result<()> {
    let node = Node::default;
    let m = || Membership::<Config>::new(vec![btreeset! {1}, btreeset! {2}], Some(btreeset! {3}));
    let ns_12 = || Some(btreemap! {1=>node(), 2=>node()});
    let ns_123 = || Some(btreemap! {1=>node(), 2=>node(), 3=>node()});
    let ns_1234 = || Some(btreemap! {1=>node(), 2=>node(), 3=>node(), 4=>node()});

    let res = m().set_nodes(None)?;
    assert_eq!(&None, res.get_nodes());

    let res = m().set_nodes(ns_123())?;
    assert_eq!(&ns_123(), res.get_nodes());

    let res = m().set_nodes(ns_1234())?;
    assert_eq!(&ns_123(), res.get_nodes());

    let res = m().set_nodes(ns_12());
    assert_eq!(
        Err(NodeIdNotInNodes {
            node_id: 3,
            node_ids: btreeset! {1,2},
        }),
        res
    );
    Ok(())
}

#[test]
fn test_membership_majority() -> anyhow::Result<()> {
    {
        let m12345 = Membership::<Config>::new(vec![btreeset! {1,2,3,4,5}], None);
        assert!(!m12345.is_majority(&btreeset! {0}));
        assert!(!m12345.is_majority(&btreeset! {0,1,2}));
        assert!(!m12345.is_majority(&btreeset! {6,7,8}));
        assert!(m12345.is_majority(&btreeset! {1,2,3}));
        assert!(m12345.is_majority(&btreeset! {3,4,5}));
        assert!(m12345.is_majority(&btreeset! {1,3,4,5}));
    }

    {
        let m12345_678 = Membership::<Config>::new(vec![btreeset! {1,2,3,4,5}, btreeset! {6,7,8}], None);
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
        let m123 = Membership::<Config>::new(vec![btreeset! {1,2,3}], None);
        assert_eq!(
            None,
            m123.greatest_majority_value(&BTreeMap::<<Config as RaftTypeConfig>::NodeId, u64>::new())
        );
        assert_eq!(None, m123.greatest_majority_value(&btreemap! {0=>10}));
        assert_eq!(None, m123.greatest_majority_value(&btreemap! {0=>10,1=>10}));
        assert_eq!(Some(&10), m123.greatest_majority_value(&btreemap! {0=>10,1=>10,2=>20}));
        assert_eq!(
            Some(&20),
            m123.greatest_majority_value(&btreemap! {0=>10,1=>10,2=>20,3=>30})
        );
    }

    {
        let m123_678 = Membership::<Config>::new(vec![btreeset! {1,2,3}, btreeset! {6,7,8}], None);
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

    let m123 = Membership::<Config>::new(vec![set123()], None);
    let m345 = Membership::<Config>::new(vec![set345()], None);
    let m123_345 = Membership::<Config>::new(vec![set123(), set345()], None);
    let m345_789 = Membership::<Config>::new(vec![set345(), set789()], None);

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

    let m1 = Membership::<Config>::new(vec![c1()], None);
    let m2 = Membership::<Config>::new(vec![c2()], None);
    let m12 = Membership::<Config>::new(vec![c1(), c2()], None);
    let m23 = Membership::<Config>::new(vec![c2(), c3()], None);

    assert_eq!(m1, m1.next_safe(c1(), false)?);
    assert_eq!(m12, m1.next_safe(c2(), false)?);
    assert_eq!(m1, m12.next_safe(c1(), false)?);
    assert_eq!(m2, m12.next_safe(c2(), false)?);
    assert_eq!(m23, m12.next_safe(c3(), false)?);

    // Turn removed members to learners

    let old_learners = || btreeset! {1, 2};
    let learners = || btreeset! {1, 2, 3, 4, 5};
    let m23_with_learners_old = Membership::<Config>::new(vec![c2(), c3()], Some(old_learners()));
    let m23_with_learners_new = Membership::<Config>::new(vec![c3()], Some(learners()));
    assert_eq!(m23_with_learners_new, m23_with_learners_old.next_safe(c3(), true)?);

    Ok(())
}

#[test]
fn test_membership_next_safe_with_nodes() -> anyhow::Result<()> {
    let node = |s: &str| Node {
        addr: s.to_string(),
        data: Default::default(),
    };

    let c1 = || btreeset! {1};
    let c2 = || btreeset! {2};

    // change from a Membership without nodes
    {
        let without_nodes = Membership::<Config>::new(vec![c1(), c2()], None);

        // [{2}, {1,2}] has all nodes info provided.

        let res = without_nodes.next_safe(btreemap! {1=>node("1"), 2=>node("2")}, false)?;
        assert_eq!(&Some(btreemap! {1=>node("1"), 2=>node("2")}), res.get_nodes());

        // joint [{2}, {1,3}] requires node info for 2

        let res = without_nodes.next_safe(btreemap! {1=>node("1"), 3=>node("3")}, false);
        assert_eq!(
            Err(NodeIdNotInNodes {
                node_id: 2,
                node_ids: btreeset! {1,3},
            }),
            res
        );

        // Changing to Membership without nodes is always OK

        let res = without_nodes.next_safe(btreeset! {5,6}, false)?;
        assert_eq!(&vec![btreeset! {2}, btreeset! {5,6}], res.get_configs());
    }

    // change from a Membership with nodes
    {
        let with_nodes = Membership::<Config>::new(vec![c1(), c2()], None)
            .set_nodes(Some(btreemap! {1=>node("1"), 2=>node("2")}))?;

        // joint [{2}, {1,2}]

        let res = with_nodes.next_safe(btreeset! {1,2}, false)?;
        assert_eq!(&Some(btreemap! {1=>node("1"), 2=>node("2")}), res.get_nodes());

        // joint [{2}, {1,3}]

        let res = with_nodes.next_safe(btreeset! {1,3}, false);
        assert_eq!(
            Err(NodeIdNotInNodes {
                node_id: 3,
                node_ids: btreeset! {1,2},
            }),
            res
        );

        // Removed to learner

        let res = with_nodes.next_safe(btreeset! {1}, true)?;
        assert_eq!(&Some(btreemap! {1=>node("1"), 2=>node("2")}), res.get_nodes());
        assert_eq!(&vec![btreeset! {1}], res.get_configs());
    }

    Ok(())
}
