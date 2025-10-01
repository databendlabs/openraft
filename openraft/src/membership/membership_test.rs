use std::collections::BTreeMap;
use std::fmt::Display;
use std::fmt::Formatter;

use maplit::btreemap;
use maplit::btreeset;

use crate::ChangeMembers;
use crate::Membership;
use crate::engine::testing::UTConfig;
use crate::error::MembershipError;
use crate::error::NodeNotFound;
use crate::error::Operation;

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct TestNode {
    pub addr: String,
    pub data: BTreeMap<String, String>,
}

impl Display for TestNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}; ", self.addr)?;
        for (i, (k, v)) in self.data.iter().enumerate() {
            if i > 0 {
                write!(f, ",")?;
            }
            write!(f, "{}:{}", k, v)?;
        }
        Ok(())
    }
}
#[test]
fn test_membership_summary() -> anyhow::Result<()> {
    let node = |addr: &str, k: &str| TestNode {
        addr: addr.to_string(),
        data: btreemap! {k.to_string() => k.to_string()},
    };

    let m = Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2}, btreeset! {3}], []);
    assert_eq!("{voters:[{1:(),2:()},{3:()}], learners:[]}", m.to_string());

    let m = Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2}, btreeset! {3}], btreeset! {4});
    assert_eq!("{voters:[{1:(),2:()},{3:()}], learners:[4:()]}", m.to_string());

    let m = Membership::<UTConfig<TestNode>>::new_unchecked(vec![btreeset! {1,2}, btreeset! {3}], btreemap! {
        1=>node("127.0.0.1", "k1"),
        2=>node("127.0.0.2", "k2"),
        3=>node("127.0.0.3", "k3"),
        4=>node("127.0.0.4", "k4"),

    });
    assert_eq!(
        r#"{voters:[{1:TestNode { addr: "127.0.0.1", data: {"k1": "k1"} },2:TestNode { addr: "127.0.0.2", data: {"k2": "k2"} }},{3:TestNode { addr: "127.0.0.3", data: {"k3": "k3"} }}], learners:[4:TestNode { addr: "127.0.0.4", data: {"k4": "k4"} }]}"#,
        m.to_string()
    );

    Ok(())
}

#[test]
fn test_membership() -> anyhow::Result<()> {
    let m1 = Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1}], []);
    let m123 = Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3}], []);
    let m123_345 = Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {3,4,5}], []);

    assert_eq!(Some(btreeset! {1}), m1.get_joint_config().first().cloned());
    assert_eq!(Some(btreeset! {1,2,3}), m123.get_joint_config().first().cloned());
    assert_eq!(Some(btreeset! {1,2,3}), m123_345.get_joint_config().first().cloned());

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

    assert!(!m123.get_joint_config().len() > 1);
    assert!(m123_345.get_joint_config().len() > 1);

    Ok(())
}

#[test]
fn test_membership_with_learners() -> anyhow::Result<()> {
    // test multi membership with learners
    {
        let m1_2 = Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1}], btreeset! {2});
        let m1_23 = m1_2.clone().change(ChangeMembers::AddNodes(btreemap! {3=>()}), true)?;

        // test learner and membership
        assert_eq!(vec![1], m1_2.voter_ids().collect::<Vec<_>>());
        assert_eq!(btreeset! {2}, m1_2.learner_ids().collect());

        assert_eq!(vec![1], m1_23.voter_ids().collect::<Vec<_>>());
        assert_eq!(vec![2, 3], m1_23.learner_ids().collect::<Vec<_>>());

        // Adding a member as learner has no effect:

        let m = m1_23.clone().change(ChangeMembers::AddNodes(btreemap! {1=>()}), true)?;
        // let m = m1_23.add_learner(1, ());
        assert_eq!(vec![1], m.voter_ids().collect::<Vec<_>>());

        // Adding a existent learner has no effect:

        let m = m1_23.change(ChangeMembers::AddNodes(btreemap! {3=>()}), true)?;
        assert_eq!(vec![1], m.voter_ids().collect::<Vec<_>>());
        assert_eq!(btreeset! {2,3}, m.learner_ids().collect());
    }

    // overlapping members and learners
    {
        let s1_2 =
            Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {5,6,7}], btreeset! {3,4,5});
        let x = s1_2.learner_ids().collect();
        assert_eq!(btreeset! {4}, x);
    }

    Ok(())
}

#[test]
fn test_membership_add_learner() -> anyhow::Result<()> {
    let node = |s: &str| TestNode {
        addr: s.to_string(),
        data: Default::default(),
    };

    let m_1_2 = Membership::<UTConfig<TestNode>>::new_unchecked(
        vec![btreeset! {1}, btreeset! {2}],
        btreemap! {1=>node("1"), 2=>node("2")},
    );

    // Add learner that presents in old cluster has no effect.

    let res = m_1_2.clone().change(ChangeMembers::AddNodes(btreemap! {1=>node("3")}), true)?;
    assert_eq!(
        Membership::<UTConfig<TestNode>>::new_unchecked(vec![btreeset! {2}], btreemap! {1=>node("1"), 2=>node("2")},),
        res
    );

    // Success to add a learner

    let m_1_2_3 = m_1_2.change(ChangeMembers::AddNodes(btreemap! {3=>node("3")}), true)?;
    assert_eq!(
        Membership::<UTConfig<TestNode>>::new_unchecked(
            vec![btreeset! {2}],
            btreemap! {1=>node("1"), 2=>node("2"), 3=>node("3")}
        ),
        m_1_2_3
    );

    Ok(())
}

#[test]
fn test_membership_update_nodes() -> anyhow::Result<()> {
    let node = |s: &str| TestNode {
        addr: s.to_string(),
        data: Default::default(),
    };

    let m_1_2 = Membership::<UTConfig<TestNode>>::new_unchecked(
        vec![btreeset! {1}, btreeset! {2}],
        btreemap! {1=>node("1"), 2=>node("2")},
    );

    let m_1_2_3 = m_1_2.change(ChangeMembers::SetNodes(btreemap! {2=>node("20"), 3=>node("30")}), true)?;
    assert_eq!(
        Membership::<UTConfig<TestNode>>::new_unchecked(
            vec![btreeset! {2}],
            btreemap! {1=>node("1"), 2=>node("20"), 3=>node("30")}
        ),
        m_1_2_3
    );

    Ok(())
}

#[test]
fn test_membership_extend_nodes() -> anyhow::Result<()> {
    let node = |s: &str| TestNode {
        addr: s.to_string(),
        data: Default::default(),
    };

    let ext = |a, b| Membership::<UTConfig<TestNode>>::extend_nodes(a, &b);

    assert_eq!(
        btreemap! {1=>node("1")},
        ext(btreemap! {1=>node("1")}, btreemap! {1=>node("2")}),
        "existent node will not change"
    );

    assert_eq!(
        btreemap! {1=>node("1"), 2=>node("2")},
        ext(btreemap! {1=>node("1")}, btreemap! {1=>node("2"), 2=>node("2")}),
    );

    Ok(())
}

#[test]
fn test_membership_new() -> anyhow::Result<()> {
    let node = TestNode::default;
    let with_nodes = |nodes| Membership::<UTConfig<TestNode>>::new(vec![btreeset! {1}, btreeset! {2}], nodes);

    let res = with_nodes(btreemap! {1=>node(), 2=>node()});
    assert!(res.is_ok());
    assert_eq!(
        res?,
        Membership::<UTConfig<TestNode>>::new_unchecked(
            vec![btreeset! {1}, btreeset! {2}],
            btreemap! {1=>node(), 2=>node()}
        )
    );

    let res = with_nodes(btreemap! {1=>node(), 2=>node(),3=>node()});
    assert!(res.is_ok());
    assert_eq!(
        res?,
        Membership::<UTConfig<TestNode>>::new_unchecked(
            vec![btreeset! {1}, btreeset! {2}],
            btreemap! {1=>node(), 2=>node(),3=>node()}
        )
    );

    let res = with_nodes(btreemap! {1=>node()});
    assert_eq!(
        res.unwrap_err(),
        MembershipError::NodeNotFound(NodeNotFound::new(2, Operation::None))
    );

    Ok(())
}
#[test]
fn test_membership_new_unchecked() -> anyhow::Result<()> {
    let node = TestNode::default;
    let new_unchecked =
        |nodes| Membership::<UTConfig<TestNode>>::new_unchecked(vec![btreeset! {1}, btreeset! {2}], nodes);

    let res = new_unchecked(btreemap! {1=>node(), 2=>node()});
    assert_eq!(
        btreemap! {1=>node(), 2=>node()},
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );

    let res = new_unchecked(btreemap! {1=>node(), 2=>node(),3=>node()});
    assert_eq!(
        btreemap! {1=>node(), 2=>node(), 3=>node()},
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );

    Ok(())
}

#[test]
fn test_membership_next_coherent() -> anyhow::Result<()> {
    let nodes = || vec![1, 2, 3, 4, 5, 6, 7, 8, 9];
    let c1 = || btreeset! {1,2,3};
    let c2 = || btreeset! {3,4,5};
    let c3 = || btreeset! {7,8,9};

    #[allow(clippy::redundant_closure)]
    let new_mem = |voter_ids, ns| Membership::<UTConfig>::new_with_defaults(voter_ids, ns);

    let m1 = new_mem(vec![c1()], nodes());
    let m12 = new_mem(vec![c1(), c2()], nodes());

    assert_eq!(m1, m1.next_coherent(c1(), false));
    assert_eq!(m12, m1.next_coherent(c2(), false));
    assert_eq!(
        new_mem(vec![c1()], vec![1, 2, 3, 6, 7, 8, 9]),
        m12.next_coherent(c1(), false)
    );
    assert_eq!(
        new_mem(vec![c2()], vec![3, 4, 5, 6, 7, 8, 9]),
        m12.next_coherent(c2(), false)
    );
    assert_eq!(
        new_mem(vec![c2(), c3()], vec![3, 4, 5, 6, 7, 8, 9]),
        m12.next_coherent(c3(), false)
    );

    // Turn removed members to learners

    let old_learners = || btreeset! {1, 2};
    let learners = || btreeset! {1, 2, 3, 4, 5};
    let m23_with_learners_old = Membership::<UTConfig>::new_with_defaults(vec![c2(), c3()], old_learners());
    let m23_with_learners_new = Membership::<UTConfig>::new_with_defaults(vec![c3()], learners());
    assert_eq!(m23_with_learners_new, m23_with_learners_old.next_coherent(c3(), true));

    Ok(())
}

#[test]
fn test_membership_next_coherent_with_nodes() -> anyhow::Result<()> {
    let node = |s: &str| TestNode {
        addr: s.to_string(),
        data: Default::default(),
    };

    let c1 = || btreeset! {1};
    let c2 = || btreeset! {2};

    let initial =
        Membership::<UTConfig<TestNode>>::new_unchecked(vec![c1(), c2()], btreemap! {1=>node("1"), 2=>node("2")});

    // joint [{2}, {1,2}]

    let res = initial.next_coherent(btreeset! {1,2}, false);
    assert_eq!(
        btreemap! {1=>node("1"), 2=>node("2")},
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );

    // Removed to learner

    let res = initial.next_coherent(btreeset! {1}, true);
    assert_eq!(
        btreemap! {1=>node("1"), 2=>node("2")},
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );
    assert_eq!(&vec![btreeset! {1}], res.get_joint_config());

    Ok(())
}

#[test]
fn test_membership_next_coherent_retain_false_keeps_existing_learners() -> anyhow::Result<()> {
    let node = |s: u64| TestNode {
        addr: s.to_string(),
        data: Default::default(),
    };

    let c1 = || btreeset! {1};
    let c2 = || btreeset! {2};

    let initial = Membership::<UTConfig<TestNode>>::new_unchecked(
        vec![c1(), c2()],
        btreemap! {1=>node(1), 2=>node(2), 3=>node(3)},
    );

    let res = initial.next_coherent(btreeset! {2}, false);
    assert_eq!(&vec![btreeset! {2}], res.get_joint_config());
    assert_eq!(
        btreemap! {2=>node(2), 3=>node(3)},
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );

    Ok(())
}
