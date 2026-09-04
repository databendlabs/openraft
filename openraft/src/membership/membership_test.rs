use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Display;
use std::fmt::Formatter;

use hegel::generators;
use hegel::generators::Generator;
use hegel::generators::PrintableGenerator;
use maplit::btreemap;
use maplit::btreeset;

use crate::ChangeMembers;
use crate::Membership;
use crate::errors::EmptyMembership;
use crate::errors::MembershipError;
use crate::errors::NodeNotFound;
use crate::errors::Operation;
use crate::quorum::QuorumSet;

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
        let m1_2 = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1}], btreeset! {2});
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
            Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {5,6,7}], btreeset! {3,4,5});
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

    let m_1_2 = Membership::<u64, TestNode>::new_unchecked(
        vec![btreeset! {1}, btreeset! {2}],
        btreemap! {1=>node("1"), 2=>node("2")},
    );

    // Add learner that presents in old cluster has no effect.

    let res = m_1_2.clone().change(ChangeMembers::AddNodes(btreemap! {1=>node("3")}), true)?;
    assert_eq!(
        Membership::<u64, TestNode>::new_unchecked(vec![btreeset! {2}], btreemap! {1=>node("1"), 2=>node("2")},),
        res
    );

    // Success to add a learner

    let m_1_2_3 = m_1_2.change(ChangeMembers::AddNodes(btreemap! {3=>node("3")}), true)?;
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

    let m_1_2_3 = m_1_2.change(ChangeMembers::SetNodes(btreemap! {2=>node("20"), 3=>node("30")}), true)?;
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
fn test_membership_next_coherent() -> anyhow::Result<()> {
    let nodes = || vec![1, 2, 3, 4, 5, 6, 7, 8, 9];
    let c1 = || btreeset! {1,2,3};
    let c2 = || btreeset! {3,4,5};
    let c3 = || btreeset! {7,8,9};

    #[allow(clippy::redundant_closure)]
    let new_mem = |voter_ids, ns| Membership::<u64, ()>::new_with_defaults(voter_ids, ns);

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
    let m23_with_learners_old = Membership::<u64, ()>::new_with_defaults(vec![c2(), c3()], old_learners());
    let m23_with_learners_new = Membership::<u64, ()>::new_with_defaults(vec![c3()], learners());
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

    let initial = Membership::<u64, TestNode>::new_unchecked(vec![c1(), c2()], btreemap! {1=>node("1"), 2=>node("2")});

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

    let initial =
        Membership::<u64, TestNode>::new_unchecked(vec![c1(), c2()], btreemap! {1=>node(1), 2=>node(2), 3=>node(3)});

    let res = initial.next_coherent(btreeset! {2}, false);
    assert_eq!(&vec![btreeset! {2}], res.get_joint_config());
    assert_eq!(
        btreemap! {2=>node(2), 3=>node(3)},
        res.nodes().map(|(nid, n)| (*nid, n.clone())).collect::<BTreeMap<_, _>>()
    );

    Ok(())
}

#[test]
fn test_membership_ensure_valid() -> anyhow::Result<()> {
    // An empty voter set.
    let res = Membership::<u64, ()>::new(vec![btreeset! {1}, btreeset! {}], btreemap! {1=>()});
    assert_eq!(Err(MembershipError::EmptyMembership(EmptyMembership {})), res);

    // A voter without node data.
    let res = Membership::<u64, ()>::new(vec![btreeset! {1,2}], btreemap! {1=>()});
    let want = MembershipError::NodeNotFound(NodeNotFound::new(2, Operation::None));
    assert_eq!(Err(want), res);

    // Zero voter sets is the value `Membership::default()` carries, so the checking constructor
    // keeps accepting it; only `ensure_quorum_defined()` rejects it.
    let m = Membership::<u64, ()>::new(vec![], btreemap! {1=>()})?;
    assert_eq!(&Vec::<BTreeSet<u64>>::new(), m.get_joint_config());
    assert_eq!(
        btreemap! {1=>()},
        m.nodes().map(|(nid, n)| (*nid, *n)).collect::<BTreeMap<_, _>>()
    );

    Ok(())
}

#[test]
fn test_membership_ensure_quorum_defined() -> anyhow::Result<()> {
    let m = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2}], []);
    assert_eq!(Ok(()), m.ensure_quorum_defined());

    let no_set = Membership::<u64, ()>::new_with_defaults(vec![], []);
    assert_eq!(Err(EmptyMembership {}), no_set.ensure_quorum_defined());

    let empty_set = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2}, btreeset! {}], []);
    assert_eq!(Err(EmptyMembership {}), empty_set.ensure_quorum_defined());

    Ok(())
}

#[test]
fn test_membership_is_direct_append_compatible_with_uniform() -> anyhow::Result<()> {
    let abc = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}], []);
    let abcx = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,4}], []);
    let bcd = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {2,3,4}], []);
    let abcxy = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,4,5}], []);

    // Equal voter sets.
    assert!(abc.is_direct_append_compatible_with(&abc));

    // One voter added, and the same transition in reverse: one voter removed.
    assert!(abc.is_direct_append_compatible_with(&abcx));
    assert!(abcx.is_direct_append_compatible_with(&abc));

    // One voter added and one removed: the symmetric difference is 2.
    assert!(!abc.is_direct_append_compatible_with(&bcd));

    // Two voters added.
    assert!(!abc.is_direct_append_compatible_with(&abcxy));

    Ok(())
}

#[test]
fn test_membership_is_direct_append_compatible_with_joint() -> anyhow::Result<()> {
    let uniform = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}], []);
    let joint = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {3,4,5}], []);

    // `{1,2,3}` is shared exactly, in both directions.
    assert!(uniform.is_direct_append_compatible_with(&joint));
    assert!(joint.is_direct_append_compatible_with(&uniform));

    // Sharing voters is not sharing a voter set: `{1,2}` is in every set below, and no set is
    // shared.
    let overlapping = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {1,2,4}], []);
    let target = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,5}], []);
    assert!(!overlapping.is_direct_append_compatible_with(&target));

    // The one-voter-difference rule applies to uniform memberships only: every set below differs
    // by one voter from its counterpart, and the transition is still rejected.
    let two_sets = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {4,5,6}], []);
    let two_sets_grown = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,7}, btreeset! {4,5,6,8}], []);
    assert!(!two_sets.is_direct_append_compatible_with(&two_sets_grown));

    Ok(())
}

#[test]
fn test_membership_is_direct_append_compatible_with_many_sets() -> anyhow::Result<()> {
    let shared = || btreeset! {1,2,3};
    let previous = Membership::<u64, ()>::new_with_defaults(vec![shared()], []);

    // The shared-set rule imposes no maximum on the number of voter sets.
    for extra_sets in [
        vec![btreeset! {4,5,6}],
        vec![btreeset! {4,5,6}, btreeset! {7,8,9}],
        vec![btreeset! {4,5,6}, btreeset! {7,8,9}, btreeset! {10,11,12}],
    ] {
        let mut configs = vec![shared()];
        configs.extend(extra_sets.clone());
        let proposed = Membership::<u64, ()>::new_with_defaults(configs, []);
        assert!(
            previous.is_direct_append_compatible_with(&proposed),
            "sharing {:?} must be accepted with extra sets {:?}",
            shared(),
            extra_sets
        );

        let mut configs = vec![btreeset! {1,2,4}];
        configs.extend(extra_sets.clone());
        let unshared = Membership::<u64, ()>::new_with_defaults(configs, []);
        assert!(
            !previous.is_direct_append_compatible_with(&unshared),
            "sharing no set must be rejected with extra sets {:?}",
            extra_sets
        );
    }

    Ok(())
}

#[test]
fn test_membership_is_direct_append_compatible_with_invalid_config() -> anyhow::Result<()> {
    let previous = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}], []);

    // A membership without any voter set is never appendable, in either direction.
    let no_set = Membership::<u64, ()>::new_with_defaults(vec![], []);
    assert!(!previous.is_direct_append_compatible_with(&no_set));
    assert!(!no_set.is_direct_append_compatible_with(&previous));

    // A membership with an empty voter set is never appendable either, even though it shares
    // `{1,2,3}` with `previous`.
    let empty_set = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}, btreeset! {}], []);
    assert!(!previous.is_direct_append_compatible_with(&empty_set));
    assert!(!empty_set.is_direct_append_compatible_with(&previous));

    Ok(())
}

#[test]
fn test_membership_find_changed_node_metadata() -> anyhow::Result<()> {
    let node = |addr: &str| TestNode {
        addr: addr.to_string(),
        data: Default::default(),
    };
    let m = |nodes| Membership::<u64, TestNode>::new_unchecked(vec![btreeset! {1,2}], nodes);

    let previous = m(btreemap! {1=>node("a"), 2=>node("b"), 3=>node("c")});

    // Every shared node id keeps its `Node`.
    let unchanged = m(btreemap! {1=>node("a"), 2=>node("b"), 3=>node("c")});
    assert_eq!(None, previous.find_changed_node_metadata(&unchanged));

    // Node 3 is removed and node 4 is added: neither is a metadata change.
    let removed_and_added = m(btreemap! {1=>node("a"), 2=>node("b"), 4=>node("d")});
    assert_eq!(None, previous.find_changed_node_metadata(&removed_and_added));

    // Voter 2 gets a different address.
    let voter_changed = m(btreemap! {1=>node("a"), 2=>node("b2"), 3=>node("c")});
    assert_eq!(Some(2), previous.find_changed_node_metadata(&voter_changed));

    // Learner 3 gets a different address.
    let learner_changed = m(btreemap! {1=>node("a"), 2=>node("b"), 3=>node("c3")});
    assert_eq!(Some(3), previous.find_changed_node_metadata(&learner_changed));

    Ok(())
}

/// Node ids below are drawn from `0..UNIVERSE` so that voter sets, learners and change requests
/// overlap often, and so that every quorum of a generated membership can be enumerated.
/// `all_quorums` walks all `2^UNIVERSE` subsets, so the bound protects the running time of the
/// tests that use it and says nothing about the sizes a `Membership` supports.
const UNIVERSE: u64 = 7;

fn universe_ids() -> Vec<u64> {
    (0..UNIVERSE).collect()
}

fn id_sets() -> impl PrintableGenerator<BTreeSet<u64>> {
    generators::subsequences(universe_ids()).map(BTreeSet::from_iter)
}

fn node_maps() -> impl PrintableGenerator<BTreeMap<u64, ()>> {
    id_sets().map(|ids| ids.into_iter().map(|id| (id, ())).collect())
}

/// A non-empty voter set.
fn configs() -> impl PrintableGenerator<BTreeSet<u64>> {
    generators::subsequences(universe_ids()).min_size(1).map(BTreeSet::from_iter)
}

/// A joint config of one or two voter sets.
fn joint_configs() -> impl PrintableGenerator<Vec<BTreeSet<u64>>> {
    generators::vecs(configs()).min_size(1).max_size(2)
}

/// The oracle for `is_quorum`: `Membership` documents a quorum as a superset of a majority of
/// every config.
fn is_majority_of_every_config(configs: &[BTreeSet<u64>], granted: &BTreeSet<u64>) -> bool {
    configs.iter().all(|config| config.intersection(granted).count() * 2 > config.len())
}

/// Every subset of the `UNIVERSE` ids that is a quorum of `configs`, one bitmask per subset.
fn all_quorums(configs: &[BTreeSet<u64>]) -> Vec<u32> {
    (0..1u32 << UNIVERSE)
        .filter(|mask| {
            let granted = (0..UNIVERSE).filter(|id| mask & (1 << id) != 0).collect();
            is_majority_of_every_config(configs, &granted)
        })
        .collect()
}

/// True if every quorum in `a` shares a node with every quorum in `b`.
fn quorums_pairwise_intersect(a: &[u32], b: &[u32]) -> bool {
    a.iter().all(|x| b.iter().all(|y| x & y != 0))
}

/// A valid membership: a joint of one or two non-empty voter sets plus a few learners, with a node
/// entry for every voter.
fn memberships() -> impl PrintableGenerator<Membership<u64, ()>> {
    hegel::compose!(|tc| {
        let configs = tc.draw(joint_configs());
        let learners = tc.draw(id_sets());

        let nodes = configs.iter().flatten().chain(learners.iter()).map(|id| (*id, ())).collect::<BTreeMap<_, _>>();

        let m = Membership::new_unchecked(configs, nodes);
        m.ensure_valid().unwrap();
        m
    })
    .print_as_debug()
}

/// One change request, drawn across every `ChangeMembers` variant. `depth` bounds the nesting of
/// `Batch` so that generation terminates.
fn changes(depth: u32) -> impl PrintableGenerator<ChangeMembers<u64, ()>> {
    let mut alternatives = vec![
        hegel::compose!(|tc| { ChangeMembers::AddVoterIds(tc.draw(id_sets())) })
            .print_as_debug()
            .boxed_printable(),
        hegel::compose!(|tc| { ChangeMembers::AddVoters(tc.draw(node_maps())) })
            .print_as_debug()
            .boxed_printable(),
        hegel::compose!(|tc| { ChangeMembers::RemoveVoters(tc.draw(id_sets())) })
            .print_as_debug()
            .boxed_printable(),
        hegel::compose!(|tc| { ChangeMembers::ReplaceAllVoters(tc.draw(id_sets())) })
            .print_as_debug()
            .boxed_printable(),
        hegel::compose!(|tc| { ChangeMembers::AddNodes(tc.draw(node_maps())) })
            .print_as_debug()
            .boxed_printable(),
        hegel::compose!(|tc| { ChangeMembers::SetNodes(tc.draw(node_maps())) })
            .print_as_debug()
            .boxed_printable(),
        hegel::compose!(|tc| { ChangeMembers::RemoveNodes(tc.draw(id_sets())) })
            .print_as_debug()
            .boxed_printable(),
        hegel::compose!(|tc| { ChangeMembers::ReplaceAllNodes(tc.draw(node_maps())) })
            .print_as_debug()
            .boxed_printable(),
    ];
    if depth > 0 {
        alternatives.push(
            hegel::compose!(|tc| { ChangeMembers::Batch(tc.draw(generators::vecs(changes(depth - 1)).max_size(3))) })
                .print_as_debug()
                .boxed_printable(),
        );
    }
    generators::one_of(alternatives)
}

/// `Membership` implements `QuorumSet` over its joint config.
#[hegel::test]
fn test_membership_is_quorum_matches_the_majority_oracle(tc: hegel::TestCase) {
    let membership = tc.draw(memberships());
    let granted = tc.draw(id_sets());

    assert_eq!(
        is_majority_of_every_config(membership.get_joint_config(), &granted),
        membership.is_quorum(granted.iter()),
        "{membership} granted {granted:?}"
    );
}

/// `next_coherent` documents the membership change loop `while curr != goal { curr =
/// curr.next_coherent(goal) }`. Two steps must be enough: the first enters a joint config, the
/// second leaves it in the uniform goal.
#[hegel::test]
fn test_next_coherent_reaches_the_goal_in_two_steps(tc: hegel::TestCase) {
    let membership = tc.draw(memberships());
    let goal = tc.draw(configs());
    let retain = tc.draw(generators::booleans());

    let step1 = membership.next_coherent(goal.clone(), retain);
    let step2 = step1.next_coherent(goal.clone(), retain);

    assert_eq!(
        &vec![goal],
        step2.get_joint_config(),
        "two steps from {membership} must reach the goal"
    );
}

/// Every membership change step must keep quorum intersection: a quorum of the old config and a
/// quorum of the new config always share a node, so the two cannot decide independently.
#[hegel::test]
fn test_next_coherent_quorums_intersect_the_previous_quorums(tc: hegel::TestCase) {
    let membership = tc.draw(memberships());
    let goal = tc.draw(configs());
    let retain = tc.draw(generators::booleans());

    let next = membership.next_coherent(goal, retain);

    assert!(
        quorums_pairwise_intersect(
            &all_quorums(membership.get_joint_config()),
            &all_quorums(next.get_joint_config())
        ),
        "quorums of {membership} and of the next step {next} must intersect"
    );
}

/// The same safety requirement for the public change path: a successful `change()` lands on a
/// config whose quorums all intersect the previous config's quorums.
#[hegel::test]
fn test_change_quorums_intersect_the_previous_quorums(tc: hegel::TestCase) {
    let membership = tc.draw(memberships());
    let change = tc.draw(changes(1));
    let retain = tc.draw(generators::booleans());

    let old_quorums = all_quorums(membership.get_joint_config());

    // A rejected change leaves the membership untouched, so there is nothing to check.
    let Ok(new) = membership.clone().change(change, retain) else {
        return;
    };

    assert!(
        quorums_pairwise_intersect(&old_quorums, &all_quorums(new.get_joint_config())),
        "quorums of {membership} and of the changed {new} must intersect"
    );
}

/// A node id in `nodes` that is not a voter is a learner.
#[hegel::test]
fn test_learner_ids_are_the_non_voter_nodes(tc: hegel::TestCase) {
    let configs = tc.draw(joint_configs());
    let nodes = tc.draw(node_maps());
    let membership = Membership::<u64, ()>::new_unchecked(configs.clone(), nodes.clone());

    let voters = configs.iter().flatten().copied().collect::<BTreeSet<_>>();
    let want = nodes.keys().filter(|id| !voters.contains(id)).copied().collect::<BTreeSet<_>>();

    assert_eq!(want, membership.learner_ids().collect::<BTreeSet<_>>());
}

/// `ensure_quorum_defined` rejects exactly the memberships whose quorum set is unusable: with no
/// voter set every node set is a quorum, and with an empty voter set none is.
#[hegel::test]
fn test_ensure_quorum_defined_iff_the_quorum_set_is_usable(tc: hegel::TestCase) {
    let configs = tc.draw(generators::vecs(id_sets()).max_size(2));
    let membership = Membership::<u64, ()>::new_with_defaults(configs, universe_ids());

    let all_voters = membership.voter_ids().collect::<BTreeSet<_>>();
    let usable = !membership.is_quorum(BTreeSet::new().iter()) && membership.is_quorum(all_voters.iter());

    assert_eq!(usable, membership.ensure_quorum_defined().is_ok(), "{membership}");
}

/// `is_direct_append_compatible_with` is documented as symmetric.
#[hegel::test]
fn test_is_direct_append_compatible_with_is_symmetric(tc: hegel::TestCase) {
    let previous = tc.draw(memberships());
    let proposed = tc.draw(memberships());

    assert_eq!(
        previous.is_direct_append_compatible_with(&proposed),
        proposed.is_direct_append_compatible_with(&previous)
    );
}

/// `is_direct_append_compatible_with` accepts a transition only when quorum intersection follows.
/// The rule is conservative, so only this direction holds.
#[hegel::test]
fn test_is_direct_append_compatible_implies_intersecting_quorums(tc: hegel::TestCase) {
    let previous = tc.draw(memberships());
    let proposed = tc.draw(memberships());

    if previous.is_direct_append_compatible_with(&proposed) {
        assert!(
            quorums_pairwise_intersect(
                &all_quorums(previous.get_joint_config()),
                &all_quorums(proposed.get_joint_config())
            ),
            "appending {proposed} directly on {previous} must keep quorums intersecting"
        );
    }
}

/// `Membership::new` accepts a config iff no voter set is empty and every voter has a node entry.
#[hegel::test]
fn test_new_accepts_iff_configs_are_non_empty_and_voters_have_nodes(tc: hegel::TestCase) {
    let configs = tc.draw(generators::vecs(id_sets()).max_size(2));
    let nodes = tc.draw(node_maps());

    let want = configs.iter().all(|c| !c.is_empty()) && configs.iter().flatten().all(|id| nodes.contains_key(id));

    assert_eq!(want, Membership::<u64, ()>::new(configs, nodes).is_ok());
}

/// A membership is persisted in the log, so it has to survive a round trip through serde.
#[cfg(feature = "serde")]
#[hegel::test]
fn test_membership_serde_roundtrip(tc: hegel::TestCase) {
    let membership = tc.draw(memberships());

    let s = serde_json::to_string(&membership).unwrap();
    assert_eq!(membership, serde_json::from_str::<Membership<u64, ()>>(&s).unwrap());
}
