use hegel::generators;
use maplit::btreeset;

use crate::quorum::coherent::Coherent;
use crate::quorum::coherent::FindCoherent;
use crate::quorum::quorum_set_test::all_quorum_masks;
use crate::quorum::quorum_set_test::draw_universe_config;
use crate::quorum::quorum_set_test::draw_universe_joint;
use crate::quorum::quorum_set_test::quorums_pairwise_intersect;

#[test]
fn test_is_coherent() -> anyhow::Result<()> {
    let s123 = || btreeset! {1,2,3};
    let s345 = || btreeset! {3,4,5};
    let s789 = || btreeset! {7,8,9};

    let j123 = vec![s123()];
    let j345 = vec![s345()];
    let j123_345 = vec![s123(), s345()];
    let j345_789 = vec![s345(), s789()];

    // Two joint configs are coherent iff they share at least one config.
    assert!(j123.is_coherent_with(&j123));
    assert!(!j123.is_coherent_with(&j345));
    assert!(j123.is_coherent_with(&j123_345));
    assert!(!j123.is_coherent_with(&j345_789));

    assert!(!j345.is_coherent_with(&j123));
    assert!(j345.is_coherent_with(&j345));
    assert!(j345.is_coherent_with(&j123_345));
    assert!(j345.is_coherent_with(&j345_789));

    assert!(j123_345.is_coherent_with(&j123));
    assert!(j123_345.is_coherent_with(&j345));
    assert!(j123_345.is_coherent_with(&j123_345));
    assert!(j123_345.is_coherent_with(&j345_789));

    assert!(!j345_789.is_coherent_with(&j123));
    assert!(j345_789.is_coherent_with(&j345));
    assert!(j345_789.is_coherent_with(&j123_345));
    assert!(j345_789.is_coherent_with(&j345_789));

    Ok(())
}

#[test]
fn test_find_coherent() -> anyhow::Result<()> {
    let s1 = || btreeset! {1,2,3};
    let s2 = || btreeset! {3,4,5};
    let s3 = || btreeset! {7,8,9};

    let j1 = vec![s1()];
    let j2 = vec![s2()];
    let j12 = vec![s1(), s2()];
    let j23 = vec![s2(), s3()];

    assert_eq!(j1, j1.find_coherent(s1()));
    assert_eq!(j12, j1.find_coherent(s2()));
    assert_eq!(j1, j12.find_coherent(s1()));
    assert_eq!(j2, j12.find_coherent(s2()));
    assert_eq!(j23, j12.find_coherent(s3()));

    Ok(())
}

/// `find_coherent` promises an intermediate quorum set `X` with `self ~ X ~ other`. Coherence is
/// checked against its definition, `∀ qᵢ ∈ A, ∀ qⱼ ∈ B: qᵢ ∩ qⱼ != ø`, over every quorum of the
/// three quorum sets. This is what makes a joint-consensus membership change safe.
#[hegel::test]
fn test_find_coherent_yields_pairwise_intersecting_quorums(tc: hegel::TestCase) {
    let current = draw_universe_joint(&tc);
    let goal = draw_universe_config(&tc);

    let intermediate = current.find_coherent(goal.clone());

    let current_quorums = all_quorum_masks(&current);
    let intermediate_quorums = all_quorum_masks(&intermediate);
    let goal_quorums = all_quorum_masks(&vec![goal.clone()]);

    assert!(
        quorums_pairwise_intersect(&current_quorums, &intermediate_quorums),
        "{current:?} must be coherent with the intermediate {intermediate:?}"
    );
    assert!(
        quorums_pairwise_intersect(&intermediate_quorums, &goal_quorums),
        "the intermediate {intermediate:?} must be coherent with the goal {goal:?}"
    );
}

/// `is_coherent_with` decides coherence by looking for a shared config. When it says yes, the
/// definition must hold: every quorum of one joint intersects every quorum of the other.
#[hegel::test]
fn test_is_coherent_with_implies_intersecting_quorums(tc: hegel::TestCase) {
    let a = draw_universe_joint(&tc);
    let mut b = draw_universe_joint(&tc);

    // Two independent joints share a config too rarely to exercise the coherent case, so half of
    // the draws copy one config across.
    if tc.draw(generators::booleans()) {
        let from = tc.draw(generators::integers::<usize>().max_value(a.len() - 1));
        let to = tc.draw(generators::integers::<usize>().max_value(b.len() - 1));
        b[to] = a[from].clone();
    }

    if a.is_coherent_with(&b) {
        assert!(
            quorums_pairwise_intersect(&all_quorum_masks(&a), &all_quorum_masks(&b)),
            "coherent joints {a:?} and {b:?} must have pairwise intersecting quorums"
        );
    }
}

/// Coherence is defined by a symmetric condition on the two quorum sets, so the predicate must be
/// symmetric too.
#[hegel::test]
fn test_is_coherent_with_is_symmetric(tc: hegel::TestCase) {
    let a = draw_universe_joint(&tc);
    let mut b = draw_universe_joint(&tc);

    if tc.draw(generators::booleans()) {
        let from = tc.draw(generators::integers::<usize>().max_value(a.len() - 1));
        let to = tc.draw(generators::integers::<usize>().max_value(b.len() - 1));
        b[to] = a[from].clone();
    }

    assert_eq!(a.is_coherent_with(&b), b.is_coherent_with(&a));
}
