use std::collections::BTreeMap;

use maplit::btreemap;
use maplit::btreeset;

use crate::error::MissingNodeInfo;
use crate::Membership;
use crate::MessageSummary;
use crate::Node;

#[test]
fn test_membership_summary() -> anyhow::Result<()> {
    let node = |addr: &str, k: &str| {
        Some(Node {
            addr: addr.to_string(),
            data: btreemap! {k.to_string() => k.to_string()},
        })
    };

    let m = Membership::<u64>::new(vec![btreeset! {1,2}, btreeset! {3}], None);
    assert_eq!("members:[{1,2},{3}],learners:[]", m.summary());

    let m = Membership::<u64>::new(vec![btreeset! {1,2}, btreeset! {3}], Some(btreeset! {4}));
    assert_eq!("members:[{1,2},{3}],learners:[4]", m.summary());

    let m = Membership::<u64>::with_nodes(vec![btreeset! {1,2}, btreeset! {3}], btreemap! {
        1=>node("127.0.0.1", "k1"),
        2=>node("127.0.0.2", "k2"),
        3=>node("127.0.0.3", "k3"),
        4=>node("127.0.0.4", "k4"),

    })?;
    assert_eq!(
        "members:[{1:{127.0.0.1; k1:k1},2:{127.0.0.2; k2:k2}},{3:{127.0.0.3; k3:k3}}],learners:[4:{127.0.0.4; k4:k4}]",
        m.summary()
    );

    Ok(())
}

#[test]
fn test_membership() -> anyhow::Result<()> {
    let m1 = Membership::<u64>::new(vec![btreeset! {1}], None);
    let m123 = Membership::<u64>::new(vec![btreeset! {1,2,3}], None);
    let m123_345 = Membership::<u64>::new(vec![btreeset! {1,2,3}, btreeset! {3,4,5}], None);

    assert_eq!(Some(btreeset! {1}), m1.get_joint_config().get(0).cloned());
    assert_eq!(Some(btreeset! {1,2,3}), m123.get_joint_config().get(0).cloned());
    assert_eq!(Some(btreeset! {1,2,3}), m123_345.get_joint_config().get(0).cloned());

    assert_eq!(None, m1.get_joint_config().get(1).cloned());
    assert_eq!(None, m123.get_joint_config().get(1).cloned());
    assert_eq!(Some(btreeset! {3,4,5}), m123_345.get_joint_config().get(1).cloned());

    assert_eq!(vec![1], m1.voter_ids().collect::<Vec<_>>());
    assert_eq!(vec![1, 2, 3], m123.voter_ids().collect::<Vec<_>>());
    assert_eq!(vec![1, 2, 3, 4, 5], m123_345.voter_ids().collect::<Vec<_>>());

    assert!(!m1.is_voter(&0));
    assert!(m1.is_voter(&1));
    assert!(m123_345.is_voter(&4));
    assert!(!m123_345.is_voter(&6));

    assert!(!m123.is_in_joint_consensus());
    assert!(m123_345.is_in_joint_consensus());

    Ok(())
}

#[test]
fn test_membership_with_learners() -> anyhow::Result<()> {
    // test multi membership with learners
    {
        let m1_2 = Membership::<u64>::new(vec![btreeset! {1}], Some(btreeset! {2}));
        let m1_23 = m1_2.add_learner(3, None)?;

        // test learner and membership
        assert_eq!(vec![1], m1_2.voter_ids().collect::<Vec<_>>());
        assert_eq!(btreeset! {2}, m1_2.learner_ids().collect());

        assert_eq!(vec![1], m1_23.voter_ids().collect::<Vec<_>>());
        assert_eq!(vec![2, 3], m1_23.learner_ids().collect::<Vec<_>>());

        // Adding a member as learner has no effect:

        let m = m1_23.add_learner(1, None)?;
        assert_eq!(vec![1], m.voter_ids().collect::<Vec<_>>());

        // Adding a existent learner has no effect:

        let m = m1_23.add_learner(3, None)?;
        assert_eq!(vec![1], m.voter_ids().collect::<Vec<_>>());
        assert_eq!(btreeset! {2,3}, m.learner_ids().collect());
    }

    // overlapping members and learners
    {
        let s1_2 = Membership::<u64>::new(vec![btreeset! {1,2,3}, btreeset! {5,6,7}], Some(btreeset! {3,4,5}));
        let x = s1_2.learner_ids().collect();
        assert_eq!(btreeset! {4}, x);
    }

    Ok(())
}

#[test]
fn test_membership_add_learner() -> anyhow::Result<()> {
    let node = |s: &str| Node {
        addr: s.to_string(),
        data: Default::default(),
    };

    let m_1_2 = Membership::<u64>::with_nodes(
        vec![btreeset! {1}, btreeset! {2}],
        btreemap! {1=>Some(node("1")), 2=>Some(node("2"))},
    )?;

    // Add learner that presents in old cluster has no effect.

    let res = m_1_2.add_learner(1, Some(node("3")))?;
    assert_eq!(m_1_2, res);

    // Success to add a learner

    let m_1_2_3 = m_1_2.add_learner(3, Some(node("3")))?;
    assert_eq!(
        Membership::<u64>::with_nodes(
            vec![btreeset! {1}, btreeset! {2}],
            btreemap! {1=>Some(node("1")), 2=>Some(node("2")), 3=>Some(node("3"))}
        )?,
        m_1_2_3
    );

    // Illegal to add node id without node info into cluster with node info.
    {
        let res = m_1_2.add_learner(3, None);
        assert_eq!(
            Err(MissingNodeInfo {
                node_id: 3,
                reason: "is None".to_string(),
            }),
            res
        );
    }

    // Illegal to add node id with node info into cluster without node info.
    {
        let m_1_2 = Membership::<u64>::new(vec![btreeset! {1}, btreeset! {2}], None);

        let res = m_1_2.add_learner(3, Some(node("3")));
        assert_eq!(
            Err(MissingNodeInfo {
                node_id: 1,
                reason: "is None".to_string(),
            }),
            res
        );
    }

    Ok(())
}

#[test]
fn test_membership_extend_nodes() -> anyhow::Result<()> {
    let node = |s: &str| Node {
        addr: s.to_string(),
        data: Default::default(),
    };

    let ext = |a, b| Membership::<u64>::extend_nodes(a, &b);

    assert_eq!(
        btreemap! {1=>None},
        ext(btreemap! {1=>None}, btreemap! {1=>Some(node("1"))}),
        "existent node will not change"
    );
    assert_eq!(
        btreemap! {1=>Some(node("1"))},
        ext(btreemap! {1=>Some(node("1"))}, btreemap! {1=>None}),
        "existent node will not change"
    );
    assert_eq!(
        btreemap! {1=>Some(node("1"))},
        ext(btreemap! {1=>Some(node("1"))}, btreemap! {1=>Some(node("2"))}),
        "existent node will not change"
    );

    assert_eq!(
        btreemap! {1=>Some(node("1")), 2=>Some(node("2"))},
        ext(
            btreemap! {1=>Some(node("1"))},
            btreemap! {1=>Some(node("2")), 2=>Some(node("2"))}
        ),
    );

    Ok(())
}

// TODO: rename
#[test]
fn test_membership_with_nodes() -> anyhow::Result<()> {
    let node = || Some(Node::default());
    let m = |nodes| Membership::<u64>::with_nodes(vec![btreeset! {1}, btreeset! {2}], nodes);

    let ns_12 = || btreemap! {1=>node(), 2=>node()};
    let ns_123 = || btreemap! {1=>node(), 2=>node(), 3=>node()};

    let res = m(btreemap! {1=>None, 2=>None})?;
    assert_eq!(
        btreemap! {1=>None, 2=>None},
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );

    let res = m(btreemap! {1=>None, 2=>None,3=>None})?;
    assert_eq!(
        btreemap! {1=>None, 2=>None, 3=>None},
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );

    let res = m(ns_12())?;
    assert_eq!(
        ns_12(),
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );

    let res = m(ns_123())?;
    assert_eq!(
        ns_123(),
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );

    // errors:
    let res = m(btreemap! {1=>None});
    assert_eq!(
        Err(MissingNodeInfo {
            node_id: 2,
            reason: "is not in cluster: [1]".to_string(),
        }),
        res
    );

    let res = m(btreemap! {1=>None,2=>node()});
    assert_eq!(
        Err(MissingNodeInfo {
            node_id: 1,
            reason: "is None".to_string(),
        }),
        res
    );

    Ok(())
}

#[test]
fn test_membership_next_safe() -> anyhow::Result<()> {
    let c1 = || btreeset! {1,2,3};
    let c2 = || btreeset! {3,4,5};
    let c3 = || btreeset! {7,8,9};

    let m1 = Membership::<u64>::new(vec![c1()], None);
    let m2 = Membership::<u64>::new(vec![c2()], None);
    let m12 = Membership::<u64>::new(vec![c1(), c2()], None);
    let m23 = Membership::<u64>::new(vec![c2(), c3()], None);

    assert_eq!(m1, m1.next_safe(c1(), false)?);
    assert_eq!(m12, m1.next_safe(c2(), false)?);
    assert_eq!(m1, m12.next_safe(c1(), false)?);
    assert_eq!(m2, m12.next_safe(c2(), false)?);
    assert_eq!(m23, m12.next_safe(c3(), false)?);

    // Turn removed members to learners

    let old_learners = || btreeset! {1, 2};
    let learners = || btreeset! {1, 2, 3, 4, 5};
    let m23_with_learners_old = Membership::<u64>::new(vec![c2(), c3()], Some(old_learners()));
    let m23_with_learners_new = Membership::<u64>::new(vec![c3()], Some(learners()));
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
        let without_nodes = Membership::<u64>::new(vec![c1(), c2()], None);

        // next_safe() can not change node info type from None to Some

        let res = without_nodes.next_safe(btreemap! {1=>node("1"), 2=>node("2")}, false)?;
        assert_eq!(
            btreemap! {1=>None, 2=>None},
            res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
        );

        // joint [{2}, {1,3}] requires node info for 2

        let res = without_nodes.next_safe(btreemap! {1=>node("1"), 3=>node("3")}, false);
        assert_eq!(
            Err(MissingNodeInfo {
                node_id: 1,
                reason: "is None".to_string(),
            }),
            res
        );

        // Changing to Membership without nodes is always OK

        let res = without_nodes.next_safe(btreeset! {5,6}, false)?;
        assert_eq!(&vec![btreeset! {2}, btreeset! {5,6}], res.get_joint_config());
    }

    // change from a Membership with nodes
    {
        let with_node_infos =
            Membership::<u64>::with_nodes(vec![c1(), c2()], btreemap! {1=>Some(node("1")), 2=>Some(node("2"))})?;

        // joint [{2}, {1,2}]

        let res = with_node_infos.next_safe(btreeset! {1,2}, false)?;
        assert_eq!(
            btreemap! {1=>Some(node("1")), 2=>Some(node("2"))},
            res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
        );

        // joint [{2}, {1,3}]

        let res = with_node_infos.next_safe(btreeset! {1,3}, false);
        assert_eq!(
            Err(MissingNodeInfo {
                node_id: 3,
                reason: "is None".to_string(),
            }),
            res
        );

        // Removed to learner

        let res = with_node_infos.next_safe(btreeset! {1}, true)?;
        assert_eq!(
            btreemap! {1=>Some(node("1")), 2=>Some(node("2"))},
            res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
        );
        assert_eq!(&vec![btreeset! {1}], res.get_joint_config());
    }

    Ok(())
}
