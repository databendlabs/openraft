use std::collections::BTreeSet;

use hegel::generators;
use maplit::btreeset;

use crate::quorum::QuorumSet;

#[test]
fn test_simple_quorum_set_impl() -> anyhow::Result<()> {
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
        let qs = vec![btreeset! {1,2,3,4,5}];

        assert!(!qs.is_quorum([0].iter()));
        assert!(!qs.is_quorum([0, 1, 2].iter()));
        assert!(!qs.is_quorum([6, 7, 8].iter()));
        assert!(qs.is_quorum([1, 2, 3].iter()));
        assert!(qs.is_quorum([3, 4, 5].iter()));
        assert!(qs.is_quorum([1, 3, 4, 5].iter()));
    }

    // Vec<BTreeSet, BTreeSet> as joint-of-majority quorum set
    {
        let qs = vec![btreeset! {1,2,3,4,5}, btreeset! {6,7,8}];

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
        let m12345 = btreeset! {1,2,3,4,5};
        assert_eq!(btreeset! {1,2,3,4,5}, m12345.ids().collect());
    }

    {
        let qs = vec![btreeset! {1,2,3,4,5}, btreeset! {4,5,6,7,8}];
        assert_eq!(btreeset! {1,2,3,4,5,6,7,8}, qs.ids().collect());
    }

    Ok(())
}

/// Node ids used by [`all_quorum_masks`].
///
/// It walks all `2^UNIVERSE` subsets, so this bound protects the running time of the tests that
/// enumerate quorums; it says nothing about the sizes a `QuorumSet` supports.
pub(crate) const UNIVERSE: u64 = 7;

/// Enumerate every subset of `0..UNIVERSE` that `qs` accepts as a quorum, one bitmask per subset.
pub(crate) fn all_quorum_masks<QS>(qs: &QS) -> Vec<u32>
where QS: QuorumSet<Id = u64> {
    (0..1u32 << UNIVERSE)
        .filter(|mask| {
            let ids = (0..UNIVERSE).filter(|id| mask & (1 << id) != 0).collect::<Vec<_>>();
            qs.is_quorum(ids.iter())
        })
        .collect()
}

/// Returns true if every quorum in `a` intersects every quorum in `b`, i.e. `a ~ b` as
/// [`Coherent`](crate::quorum::Coherent) defines it.
pub(crate) fn quorums_pairwise_intersect(a: &[u32], b: &[u32]) -> bool {
    a.iter().all(|x| b.iter().all(|y| x & y != 0))
}

/// A non-empty voter set drawn from the [`UNIVERSE`] ids.
pub(crate) fn draw_universe_config(tc: &hegel::TestCase) -> BTreeSet<u64> {
    let ids = tc.draw(generators::subsequences((0..UNIVERSE).collect::<Vec<_>>()).min_size(1));
    ids.into_iter().collect()
}

/// A joint quorum set of one or two voter sets drawn from the [`UNIVERSE`] ids.
pub(crate) fn draw_universe_joint(tc: &hegel::TestCase) -> Vec<BTreeSet<u64>> {
    let count = tc.draw(generators::integers::<usize>().min_value(1).max_value(2));
    (0..count).map(|_| draw_universe_config(tc)).collect()
}

/// Node ids come mostly from a small pool so that configs and granted sets overlap often, and
/// full-range draws cover the `u64` boundaries.
///
/// `#[hegel::composite]` turns this and the two functions below into generators: callers write
/// `tc.draw(node_id())` or `tc.draw(id_set(1))`, and `draw` passes `tc` in as the first argument.
#[hegel::composite]
fn node_id(tc: &hegel::TestCase) -> u64 {
    tc.draw(hegel::one_of!(
        generators::integers::<u64>().max_value(9),
        generators::integers::<u64>(),
    ))
}

#[hegel::composite]
fn id_set(tc: &hegel::TestCase, min_size: usize) -> BTreeSet<u64> {
    tc.draw(generators::vecs(node_id()).min_size(min_size).max_size(9)).into_iter().collect()
}

/// A joint quorum set of one to three non-empty configs, the shape `Membership` builds.
#[hegel::composite]
fn joint_config(tc: &hegel::TestCase) -> Vec<BTreeSet<u64>> {
    let count = tc.draw(generators::integers::<usize>().min_value(1).max_value(3));
    (0..count).map(|_| tc.draw(id_set(1))).collect()
}

/// `BTreeSet` is documented as a simple majority quorum set: strictly more than half of the
/// members must be granted. The oracle counts the overlap in one pass, where `is_quorum` counts
/// incrementally and returns early.
#[hegel::test]
fn test_majority_quorum_matches_counting_oracle(tc: hegel::TestCase) {
    let config: BTreeSet<u64> = tc.draw(id_set(0));
    let granted: BTreeSet<u64> = tc.draw(id_set(0));

    let want = config.intersection(&granted).count() * 2 > config.len();
    assert_eq!(want, config.is_quorum(granted.iter()));
}

/// The `&[ID]` and `BTreeSet<ID>` implementations are both documented as simple majority quorum
/// sets, so over the same members they must answer alike.
#[hegel::test]
fn test_slice_quorum_set_agrees_with_btreeset(tc: hegel::TestCase) {
    let config: BTreeSet<u64> = tc.draw(id_set(0));
    let granted: BTreeSet<u64> = tc.draw(id_set(0));

    let slice = config.iter().copied().collect::<Vec<_>>();
    assert_eq!(
        config.is_quorum(granted.iter()),
        slice.as_slice().is_quorum(granted.iter())
    );
}

/// A joint quorum set grants iff the granted ids are a quorum in every one of its configs.
///
/// The per-config answer comes from `BTreeSet::is_quorum`; what is under test here is the joint
/// combination rule, and the majority rule itself is pinned by
/// `test_majority_quorum_matches_counting_oracle`.
#[hegel::test]
fn test_joint_quorum_is_a_quorum_in_every_config(tc: hegel::TestCase) {
    let joint: Vec<BTreeSet<u64>> = tc.draw(joint_config());
    let granted: BTreeSet<u64> = tc.draw(id_set(0));

    let want = joint.iter().all(|config| config.is_quorum(granted.iter()));
    assert_eq!(want, joint.is_quorum(granted.iter()));
}

/// The `QuorumSet` trait requires implementations to be upward-closed: adding ids to a quorum
/// must leave it a quorum.
#[hegel::test]
fn test_quorum_set_is_upward_closed(tc: hegel::TestCase) {
    let joint: Vec<BTreeSet<u64>> = tc.draw(joint_config());

    // Granted ids are drawn from the members, otherwise quorums would be too rare to exercise
    // the implication.
    let members = joint.ids().collect::<Vec<_>>();
    let granted = tc.draw(generators::subsequences(members)).into_iter().collect::<BTreeSet<_>>();
    let extra: BTreeSet<u64> = tc.draw(id_set(0));
    let bigger = granted.union(&extra).copied().collect::<BTreeSet<_>>();

    assert!(
        !joint.is_quorum(granted.iter()) || joint.is_quorum(bigger.iter()),
        "adding {extra:?} to quorum {granted:?} of {joint:?} must leave it a quorum"
    );
}

/// `ids()` returns all ids in the quorum set: the union of its configs, ascending and deduplicated.
#[hegel::test]
fn test_joint_ids_is_the_union_of_configs(tc: hegel::TestCase) {
    let joint: Vec<BTreeSet<u64>> = tc.draw(joint_config());

    let want = joint.iter().flatten().copied().collect::<BTreeSet<_>>();
    assert_eq!(want.into_iter().collect::<Vec<_>>(), joint.ids().collect::<Vec<_>>());
}

/// The safety requirement Raft places on a quorum set: any two quorums share a node, so two
/// conflicting decisions cannot both be granted. Checked over every quorum of the generated
/// config.
#[hegel::test]
fn test_two_quorums_of_a_joint_config_intersect(tc: hegel::TestCase) {
    let joint = draw_universe_joint(&tc);

    let quorums = all_quorum_masks(&joint);
    assert!(
        quorums_pairwise_intersect(&quorums, &quorums),
        "two quorums of {joint:?} must intersect"
    );
}
