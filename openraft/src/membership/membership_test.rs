use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Display;
use std::fmt::Formatter;

use maplit::btreemap;
use maplit::btreeset;
use quorum_set::verify_intersection;

use crate::ChangeMembers;
use crate::Membership;
use crate::errors::EmptyMembership;
use crate::errors::MembershipError;
use crate::errors::NodeMetadataChanged;
use crate::errors::NodeNotFound;
use crate::errors::Operation;
use crate::membership::RaftMembership;
use crate::membership::RaftMembershipExt;

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

    let m = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2}, btreeset! {3}], []);
    assert_eq!("{voters:[{1:(),2:()},{3:()}], learners:[]}", m.to_string());

    let m = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2}, btreeset! {3}], btreeset! {4});
    assert_eq!("{voters:[{1:(),2:()},{3:()}], learners:[4:()]}", m.to_string());

    let m = Membership::<u64, TestNode>::new_unchecked(vec![btreeset! {1,2}, btreeset! {3}], btreemap! {
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
    let m1 = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1}], []);
    let m123 = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}], []);
    let m123_345 = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {3,4,5}], []);

    assert_eq!(Some(btreeset! {1}), m1.get_joint_config().first().cloned());
    assert_eq!(Some(btreeset! {1,2,3}), m123.get_joint_config().first().cloned());
    assert_eq!(Some(btreeset! {1,2,3}), m123_345.get_joint_config().first().cloned());

    assert_eq!(None, m1.get_joint_config().get(1).cloned());
    assert_eq!(None, m123.get_joint_config().get(1).cloned());
    assert_eq!(Some(btreeset! {3,4,5}), m123_345.get_joint_config().get(1).cloned());

    assert_eq!(vec![1], m1.voter_ids().collect::<Vec<_>>());
    assert_eq!(vec![1, 2, 3], m123.voter_ids().collect::<Vec<_>>());
    assert_eq!(vec![1, 2, 3, 4, 5], m123_345.voter_ids().collect::<Vec<_>>());

    assert!(!m123.get_joint_config().len() > 1);
    assert!(m123_345.get_joint_config().len() > 1);

    Ok(())
}

#[test]
fn test_membership_roles() {
    let cases = [
        (vec![], btreeset! {}, btreeset! {1,3,6}),
        (vec![btreeset! {1}], btreeset! {1}, btreeset! {3,6}),
        (
            vec![btreeset! {1,2,3}, btreeset! {3,4,5}],
            btreeset! {1,2,3,4,5},
            btreeset! {6},
        ),
    ];
    for (configs, voters, learners) in cases {
        let membership = Membership::<u64, ()>::new_with_defaults(configs, [1, 3, 6]);
        for id in 0..=7 {
            let expected = voters.contains(&id);
            let is_campaigner = membership.is_campaigner(&id);
            assert_eq!(expected, is_campaigner, "campaigner {id}");
            let is_read_voter = membership.is_read_voter(&id);
            assert_eq!(expected, is_read_voter, "read voter {id}");
            let is_write_voter = membership.is_write_voter(&id);
            assert_eq!(expected, is_write_voter, "write voter {id}");
            let expected_learner = learners.contains(&id);
            let is_learner = membership.is_learner(&id);
            assert_eq!(expected_learner, is_learner, "learner {id}");
        }
    }
}

#[test]
fn test_membership_with_learners() -> anyhow::Result<()> {
    // test multi membership with learners
    {
        let m1_2 = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1}], btreeset! {2});
        let m1_23 = m1_2.clone().change(ChangeMembers::AddNodes(btreemap! {3=>()}))?;

        // test learner and membership
        assert_eq!(vec![1], m1_2.voter_ids().collect::<Vec<_>>());
        assert_eq!(btreeset! {2}, m1_2.non_write_voter_ids().collect());

        assert_eq!(vec![1], m1_23.voter_ids().collect::<Vec<_>>());
        assert_eq!(vec![2, 3], m1_23.non_write_voter_ids().collect::<Vec<_>>());

        // Adding a member as learner has no effect:

        let m = m1_23.clone().change(ChangeMembers::AddNodes(btreemap! {1=>()}))?;
        // let m = m1_23.add_learner(1, ());
        assert_eq!(vec![1], m.voter_ids().collect::<Vec<_>>());

        // Adding a existent learner has no effect:

        let m = m1_23.change(ChangeMembers::AddNodes(btreemap! {3=>()}))?;
        assert_eq!(vec![1], m.voter_ids().collect::<Vec<_>>());
        assert_eq!(btreeset! {2,3}, m.non_write_voter_ids().collect());
    }

    // overlapping members and learners
    {
        let s1_2 =
            Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {5,6,7}], btreeset! {3,4,5});
        let x = s1_2.non_write_voter_ids().collect();
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

    let m_1_2 = Membership::<u64, TestNode>::new_unchecked(
        vec![btreeset! {1}, btreeset! {2}],
        btreemap! {1=>node("1"), 2=>node("2")},
    );

    // Add learner that presents in old cluster has no effect.

    let res = m_1_2.clone().change(ChangeMembers::AddNodes(btreemap! {1=>node("3")}))?;
    assert_eq!(
        Membership::<u64, TestNode>::new_unchecked(vec![btreeset! {2}], btreemap! {1=>node("1"), 2=>node("2")},),
        res
    );

    // Success to add a learner

    let m_1_2_3 = m_1_2.change(ChangeMembers::AddNodes(btreemap! {3=>node("3")}))?;
    assert_eq!(
        Membership::<u64, TestNode>::new_unchecked(
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

    let m_1_2 = Membership::<u64, TestNode>::new_unchecked(
        vec![btreeset! {1}, btreeset! {2}],
        btreemap! {1=>node("1"), 2=>node("2")},
    );

    let m_1_2_3 = m_1_2.change(ChangeMembers::SetNodes(btreemap! {2=>node("20"), 3=>node("30")}))?;
    assert_eq!(
        Membership::<u64, TestNode>::new_unchecked(
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

    let ext = |a, b| Membership::<u64, TestNode>::extend_nodes(a, &b);

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
    let with_nodes = |nodes| Membership::<u64, TestNode>::new(vec![btreeset! {1}, btreeset! {2}], nodes);

    let res = with_nodes(btreemap! {1=>node(), 2=>node()});
    assert!(res.is_ok());
    assert_eq!(
        res?,
        Membership::<u64, TestNode>::new_unchecked(
            vec![btreeset! {1}, btreeset! {2}],
            btreemap! {1=>node(), 2=>node()}
        )
    );

    let res = with_nodes(btreemap! {1=>node(), 2=>node(),3=>node()});
    assert!(res.is_ok());
    assert_eq!(
        res?,
        Membership::<u64, TestNode>::new_unchecked(
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
    let new_unchecked = |nodes| Membership::<u64, TestNode>::new_unchecked(vec![btreeset! {1}, btreeset! {2}], nodes);

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
fn test_membership_joint_with() -> anyhow::Result<()> {
    let node = |s: &str| TestNode {
        addr: s.to_string(),
        data: Default::default(),
    };

    let new = Membership::<u64, TestNode>::new_unchecked(
        vec![btreeset! {3,4,5}],
        btreemap! {3=>node("30"), 4=>node("4"), 5=>node("5")},
    );

    // Node 3 takes its metadata from `new`; learner 6 of `old` is dropped.
    let old = Membership::<u64, TestNode>::new_unchecked(
        vec![btreeset! {1,2,3}],
        btreemap! {1=>node("1"), 2=>node("2"), 3=>node("3"), 6=>node("6")},
    );
    let joint = old.joint_with(&new);
    let want = Membership::<u64, TestNode>::new_unchecked(
        vec![btreeset! {1,2,3}, btreeset! {3,4,5}],
        btreemap! {1=>node("1"), 2=>node("2"), 3=>node("30"), 4=>node("4"), 5=>node("5")},
    );
    assert_eq!(want, joint);

    Ok(())
}

#[test]
fn test_membership_change_with_nodes() -> anyhow::Result<()> {
    let node = |s: &str| TestNode {
        addr: s.to_string(),
        data: Default::default(),
    };

    let c1 = || btreeset! {1};
    let c2 = || btreeset! {2};

    let initial = Membership::<u64, TestNode>::new_unchecked(vec![c1(), c2()], btreemap! {1=>node("1"), 2=>node("2")});

    // uniform [{1,2}]

    let res = initial.change(ChangeMembers::ReplaceAllVoters(btreeset! {1,2}))?;
    assert_eq!(
        btreemap! {1=>node("1"), 2=>node("2")},
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );
    assert_eq!(&vec![btreeset! {1,2}], res.get_joint_config());

    // Removed to learner

    let res = initial.change(ChangeMembers::ReplaceAllVoters(btreeset! {1}))?;
    assert_eq!(
        btreemap! {1=>node("1"), 2=>node("2")},
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );
    assert_eq!(&vec![btreeset! {1}], res.get_joint_config());

    Ok(())
}

#[test]
fn test_membership_drop_voters_demoted_to_learners_keeps_existing_learners() -> anyhow::Result<()> {
    let node = |s: u64| TestNode {
        addr: s.to_string(),
        data: Default::default(),
    };

    let c1 = || btreeset! {1};
    let c2 = || btreeset! {2};

    let initial =
        Membership::<u64, TestNode>::new_unchecked(vec![c1(), c2()], btreemap! {1=>node(1), 2=>node(2), 3=>node(3)});

    let goal = initial.change(ChangeMembers::ReplaceAllVoters(btreeset! {2}))?;
    let res = initial.drop_voters_demoted_to_learners(goal);
    assert_eq!(&vec![btreeset! {2}], res.get_joint_config());
    assert_eq!(
        btreemap! {2=>node(2), 3=>node(3)},
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );

    Ok(())
}

#[test]
fn test_membership_new_checks_only_voter_nodes() -> anyhow::Result<()> {
    // An empty voter set passes; OpenRaft checks quorums before it writes a membership to the log.
    let m = Membership::<u64, ()>::new(vec![btreeset! {1}, btreeset! {}], btreemap! {1=>()})?;
    let want = Membership::<u64, ()>::new_unchecked(vec![btreeset! {1}, btreeset! {}], btreemap! {1=>()});
    assert_eq!(want, m);

    // A voter without node data.
    let res = Membership::<u64, ()>::new(vec![btreeset! {1,2}], btreemap! {1=>()});
    let want = MembershipError::NodeNotFound(NodeNotFound::new(2, Operation::None));
    assert_eq!(Err(want), res);

    // Zero voter sets is the value `Membership::default()` carries, so the checking constructor
    // keeps accepting it; only `ensure_non_empty_config()` rejects it.
    let m = Membership::<u64, ()>::new(vec![], btreemap! {1=>()})?;
    assert_eq!(&Vec::<BTreeSet<u64>>::new(), m.get_joint_config());
    assert_eq!(
        btreemap! {1=>()},
        m.nodes().map(|(nid, n)| (*nid, *n)).collect::<BTreeMap<_, _>>()
    );

    Ok(())
}

#[test]
fn test_membership_ensure_non_empty_config() -> anyhow::Result<()> {
    let m = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2}], []);
    let res = m.ensure_non_empty_config();
    assert_eq!(Ok(()), res);

    let no_set = Membership::<u64, ()>::new_with_defaults(vec![], []);
    let res = no_set.ensure_non_empty_config();
    assert_eq!(Err(EmptyMembership {}), res);

    let empty_set = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2}, btreeset! {}], []);
    let res = empty_set.ensure_non_empty_config();
    assert_eq!(Err(EmptyMembership {}), res);

    Ok(())
}

#[test]
fn test_membership_supports_direct_transition_to_uniform() -> anyhow::Result<()> {
    let abc = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}], []);
    let abcx = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,4}], []);
    let abcy = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,5}], []);
    let bcd = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {2,3,4}], []);
    let abcxy = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,4,5}], []);

    // Equal voter sets.
    assert!(abc.supports_direct_transition_to(&abc));

    // One voter added, and the same transition in reverse: one voter removed.
    assert!(abc.supports_direct_transition_to(&abcx));
    assert!(abcx.supports_direct_transition_to(&abc));

    // One of three voters replaced: majority `{1,2}` of `{1,2,3}` misses majority `{3,4}` of
    // `{2,3,4}`.
    assert!(!abc.supports_direct_transition_to(&bcd));

    // One of four voters replaced: a majority of either set holds at least two of `{1,2,3}`, so
    // the two majorities intersect.
    assert!(abcx.supports_direct_transition_to(&abcy));

    // Two voters added: majority `{1,2}` of `{1,2,3}` misses majority `{3,4,5}` of `{1,2,3,4,5}`.
    assert!(!abc.supports_direct_transition_to(&abcxy));

    Ok(())
}

#[test]
fn test_membership_supports_direct_transition_to_joint() -> anyhow::Result<()> {
    let uniform = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}], []);
    let joint = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {3,4,5}], []);

    // `{1,2,3}` is shared exactly, in both directions.
    assert!(uniform.supports_direct_transition_to(&joint));
    assert!(joint.supports_direct_transition_to(&uniform));

    // Sharing voters is not enough: `{1,2}` is in every set below, but no pair of sets has
    // intersecting majorities. Majority `{1,3}` of `{1,2,3}` misses majority `{2,5}` of `{1,2,5}`.
    let overlapping = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {1,2,4}], []);
    let target = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,5}], []);
    assert!(!overlapping.supports_direct_transition_to(&target));

    // Every set below differs by one voter from its counterpart. A quorum of `two_sets` holds a
    // majority of `{1,2,3}`, a quorum of `two_sets_grown` holds a majority of `{1,2,3,7}`, and
    // these two majorities always intersect.
    let two_sets = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {4,5,6}], []);
    let two_sets_grown = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,7}, btreeset! {4,5,6,8}], []);
    assert!(two_sets.supports_direct_transition_to(&two_sets_grown));

    Ok(())
}

#[test]
fn test_membership_supports_direct_transition_to_many_sets() -> anyhow::Result<()> {
    let shared = || btreeset! {1,2,3};
    let previous = Membership::<u64, ()>::new_with_defaults(vec![shared()], []);

    // The transition rule imposes no maximum on the number of voter sets.
    for extra_sets in [
        vec![btreeset! {4,5,6}],
        vec![btreeset! {4,5,6}, btreeset! {7,8,9}],
        vec![btreeset! {4,5,6}, btreeset! {7,8,9}, btreeset! {10,11,12}],
    ] {
        let mut configs = vec![shared()];
        configs.extend(extra_sets.clone());
        let proposed = Membership::<u64, ()>::new_with_defaults(configs, []);
        assert!(
            previous.supports_direct_transition_to(&proposed),
            "sharing {:?} must be accepted with extra sets {:?}",
            shared(),
            extra_sets
        );

        let mut configs = vec![btreeset! {1,2,4}];
        configs.extend(extra_sets.clone());
        let unshared = Membership::<u64, ()>::new_with_defaults(configs, []);
        assert!(
            !previous.supports_direct_transition_to(&unshared),
            "sharing no set must be rejected with extra sets {:?}",
            extra_sets
        );
    }

    Ok(())
}

#[test]
fn test_membership_supports_direct_transition_to_invalid_config() -> anyhow::Result<()> {
    let previous = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}], []);

    // A membership without any voter set is never appendable, in either direction.
    let no_set = Membership::<u64, ()>::new_with_defaults(vec![], []);
    assert!(!previous.supports_direct_transition_to(&no_set));
    assert!(!no_set.supports_direct_transition_to(&previous));

    // A membership with an empty voter set is never appendable either, even though it shares
    // `{1,2,3}` with `previous`.
    let empty_set = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {}], []);
    assert!(!previous.supports_direct_transition_to(&empty_set));
    assert!(!empty_set.supports_direct_transition_to(&previous));

    Ok(())
}

#[test]
fn test_membership_ensure_node_metadata_unchanged() -> anyhow::Result<()> {
    let node = |addr: &str| TestNode {
        addr: addr.to_string(),
        data: Default::default(),
    };
    let m = |nodes| Membership::<u64, TestNode>::new_unchecked(vec![btreeset! {1,2}], nodes);

    let previous = m(btreemap! {1=>node("a"), 2=>node("b"), 3=>node("c")});

    // Every shared node id keeps its `Node`.
    let unchanged = m(btreemap! {1=>node("a"), 2=>node("b"), 3=>node("c")});
    let res = previous.ensure_node_metadata_unchanged(&unchanged);
    assert_eq!(Ok(()), res);

    // Node 3 is removed and node 4 is added: neither is a metadata change.
    let removed_and_added = m(btreemap! {1=>node("a"), 2=>node("b"), 4=>node("d")});
    let res = previous.ensure_node_metadata_unchanged(&removed_and_added);
    assert_eq!(Ok(()), res);

    // Voter 2 gets a different address.
    let voter_changed = m(btreemap! {1=>node("a"), 2=>node("b2"), 3=>node("c")});
    let res = previous.ensure_node_metadata_unchanged(&voter_changed);
    assert_eq!(Err(NodeMetadataChanged { node_id: 2 }), res);

    // Learner 3 gets a different address.
    let learner_changed = m(btreemap! {1=>node("a"), 2=>node("b"), 3=>node("c3")});
    let res = previous.ensure_node_metadata_unchanged(&learner_changed);
    assert_eq!(Err(NodeMetadataChanged { node_id: 3 }), res);

    Ok(())
}

/// `supports_direct_transition_to` accepts a transition between two uniform memberships exactly
/// when every quorum of one intersects every quorum of the other. Checked against the definition
/// over every pair of non-empty voter sets over five ids. Joint memberships are left out: the
/// check is conservative for them, and the `quorum-set` crate tests it against the same
/// definition.
#[test]
fn test_supports_direct_transition_is_exact_for_uniform() {
    const UNIVERSE: u64 = 5;

    let configs = (1..1u64 << UNIVERSE)
        .map(|mask| (0..UNIVERSE).filter(|id| mask & (1 << id) != 0).collect::<BTreeSet<_>>())
        .collect::<Vec<_>>();
    let memberships = configs
        .into_iter()
        .map(|config| Membership::<u64, ()>::new_with_defaults(vec![config], []))
        .collect::<Vec<_>>();

    for previous in &memberships {
        for proposed in &memberships {
            let compatible = previous.supports_direct_transition_to(proposed);
            let intersecting = verify_intersection(previous.get_joint_config(), proposed.get_joint_config());
            assert_eq!(
                intersecting, compatible,
                "appending {proposed} directly on {previous} must be accepted iff quorums intersect"
            );
        }
    }
}
