//! Property-based tests (hegel) for the `Vote` ordering semantics.
//!
//! Vote ordering carries Raft's safety argument: a vote is granted only when it is greater than
//! the stored one, so `PartialOrd` has to be a real partial order. The laws below are checked over
//! both leader-id flavors, which order differently by design: `leader_id_adv` is totally ordered,
//! `leader_id_std` leaves same-term different-node leader ids incomparable.

use std::cmp::Ordering;

use hegel::generators;
use hegel::generators::Generator;
use hegel::generators::PrintableGenerator;

use crate::Vote;
use crate::vote::RaftLeaderId;
use crate::vote::leader_id_adv;
use crate::vote::leader_id_std;

type StdLeaderId = leader_id_std::LeaderId<u64, u64>;
type AdvLeaderId = leader_id_adv::LeaderId<u64, u64>;

/// Terms and node ids are mostly drawn from a three-value pool, so that the equal terms and equal
/// node ids the comparison rules turn on are common, and full-range draws cover the `u64`
/// boundaries.
#[hegel::composite]
fn term_or_node_id(tc: &hegel::TestCase) -> u64 {
    tc.draw(hegel::one_of!(
        generators::integers::<u64>().max_value(2),
        generators::integers::<u64>(),
    ))
}

fn votes<LID>() -> impl PrintableGenerator<Vote<LID>>
where LID: RaftLeaderId<Term = u64, NodeId = u64> {
    hegel::compose!(|tc| {
        let term = tc.draw(term_or_node_id());
        let node_id = tc.draw(term_or_node_id());

        if tc.draw(generators::booleans()) {
            Vote::new_committed(term, node_id)
        } else {
            Vote::new(term, node_id)
        }
    })
    .print_as_debug()
}

#[hegel::test]
fn test_vote_partial_cmp_is_reflexive(tc: hegel::TestCase) {
    let std_vote = tc.draw(votes::<StdLeaderId>());
    assert_eq!(Some(Ordering::Equal), std_vote.partial_cmp(&std_vote), "{std_vote}");

    let adv_vote = tc.draw(votes::<AdvLeaderId>());
    assert_eq!(Some(Ordering::Equal), adv_vote.partial_cmp(&adv_vote), "{adv_vote}");
}

/// Comparing two votes the other way round yields the reversed answer, including the incomparable
/// case: `PartialOrd` requires `a < b` iff `b > a`.
#[hegel::test]
fn test_vote_partial_cmp_is_antisymmetric(tc: hegel::TestCase) {
    let std_a = tc.draw(votes::<StdLeaderId>());
    let std_b = tc.draw(votes::<StdLeaderId>());
    assert_eq!(
        std_a.partial_cmp(&std_b),
        std_b.partial_cmp(&std_a).map(Ordering::reverse),
        "{std_a} vs {std_b}"
    );

    let adv_a = tc.draw(votes::<AdvLeaderId>());
    let adv_b = tc.draw(votes::<AdvLeaderId>());
    assert_eq!(
        adv_a.partial_cmp(&adv_b),
        adv_b.partial_cmp(&adv_a).map(Ordering::reverse),
        "{adv_a} vs {adv_b}"
    );
}

/// `partial_cmp` reports `Equal` exactly for the votes `PartialEq` considers equal.
#[hegel::test]
fn test_vote_partial_cmp_equal_agrees_with_eq(tc: hegel::TestCase) {
    let std_a = tc.draw(votes::<StdLeaderId>());
    let std_b = tc.draw(votes::<StdLeaderId>());
    assert_eq!(
        std_a == std_b,
        std_a.partial_cmp(&std_b) == Some(Ordering::Equal),
        "{std_a} vs {std_b}"
    );

    let adv_a = tc.draw(votes::<AdvLeaderId>());
    let adv_b = tc.draw(votes::<AdvLeaderId>());
    assert_eq!(
        adv_a == adv_b,
        adv_a.partial_cmp(&adv_b) == Some(Ordering::Equal),
        "{adv_a} vs {adv_b}"
    );
}

/// `a <= b` and `b <= c` imply `a <= c`, which is what lets vote-granting decisions chain. The
/// "committed wins between incomparable leader ids" rule in `RefVote::partial_cmp` is where it
/// could break.
#[hegel::test]
fn test_vote_partial_cmp_is_transitive(tc: hegel::TestCase) {
    let std_a = tc.draw(votes::<StdLeaderId>());
    let std_b = tc.draw(votes::<StdLeaderId>());
    let std_c = tc.draw(votes::<StdLeaderId>());
    if std_a <= std_b && std_b <= std_c {
        assert!(
            std_a <= std_c,
            "{std_a} <= {std_b} <= {std_c} but not {std_a} <= {std_c}"
        );
    }

    let adv_a = tc.draw(votes::<AdvLeaderId>());
    let adv_b = tc.draw(votes::<AdvLeaderId>());
    let adv_c = tc.draw(votes::<AdvLeaderId>());
    if adv_a <= adv_b && adv_b <= adv_c {
        assert!(
            adv_a <= adv_c,
            "{adv_a} <= {adv_b} <= {adv_c} but not {adv_a} <= {adv_c}"
        );
    }
}

/// `leader_id_adv` is documented as totally ordered, so no pair of votes over it is incomparable.
#[hegel::test]
fn test_adv_votes_are_never_incomparable(tc: hegel::TestCase) {
    let adv_a = tc.draw(votes::<AdvLeaderId>());
    let adv_b = tc.draw(votes::<AdvLeaderId>());

    assert!(adv_a.partial_cmp(&adv_b).is_some(), "{adv_a} vs {adv_b}");
}

/// With `leader_id_adv`, votes order lexicographically by term, then node id, then commit status.
#[hegel::test]
fn test_adv_vote_order_matches_the_tuple_oracle(tc: hegel::TestCase) {
    let adv_a = tc.draw(votes::<AdvLeaderId>());
    let adv_b = tc.draw(votes::<AdvLeaderId>());

    let key = |v: &Vote<AdvLeaderId>| (v.leader_id.term, v.leader_id.node_id, v.committed);
    assert_eq!(
        Some(key(&adv_a).cmp(&key(&adv_b))),
        adv_a.partial_cmp(&adv_b),
        "{adv_a} vs {adv_b}"
    );
}

/// Standard Raft allows at most one leader per term: with `leader_id_std`, two uncommitted votes
/// of the same term for different nodes are incomparable, so neither can overwrite the other.
#[hegel::test]
fn test_std_votes_of_one_term_for_different_nodes_are_incomparable(tc: hegel::TestCase) {
    let term = tc.draw(term_or_node_id());
    let node_a = tc.draw(term_or_node_id());
    let node_b = tc.draw(term_or_node_id());
    tc.assume(node_a != node_b);

    let a = Vote::<StdLeaderId>::new(term, node_a);
    let b = Vote::<StdLeaderId>::new(term, node_b);

    assert_eq!(None, a.partial_cmp(&b), "{a} vs {b}");
}

/// With `leader_id_std`, a committed vote overrides any uncommitted vote of the same or a lower
/// term, so that a granted leader is not displaced by a candidate that cannot win.
#[hegel::test]
fn test_std_committed_vote_beats_an_uncommitted_vote_of_no_higher_term(tc: hegel::TestCase) {
    let committed_term = tc.draw(term_or_node_id());
    let committed_node_id = tc.draw(term_or_node_id());
    let uncommitted_term = tc.draw(generators::integers::<u64>().max_value(committed_term));
    let uncommitted_node_id = tc.draw(term_or_node_id());

    let committed = Vote::<StdLeaderId>::new_committed(committed_term, committed_node_id);
    let uncommitted = Vote::<StdLeaderId>::new(uncommitted_term, uncommitted_node_id);

    assert!(committed > uncommitted, "{committed} must beat {uncommitted}");
}

/// Every `Vote` and `LeaderId` field is public, so votes can be built by struct literal rather than
/// through the constructors.
fn publicly_built_std_votes() -> impl PrintableGenerator<Vote<StdLeaderId>> {
    hegel::compose!(|tc| {
        Vote {
            leader_id: StdLeaderId {
                term: tc.draw(term_or_node_id()),
                voted_for: tc.draw(term_or_node_id()),
            },
            committed: tc.draw(generators::booleans()),
        }
    })
    .print_as_debug()
}

/// Regression witness for the comparison panic reported in databendlabs/openraft#1872 and fixed in
/// #1874: with `leader_id_std`, comparing two equal-term votes reached
/// `LeaderId::voted_for.unwrap()` whenever either side held `None`.
#[hegel::test]
fn test_comparing_publicly_built_std_votes_does_not_panic(tc: hegel::TestCase) {
    let std_a = tc.draw(publicly_built_std_votes());
    let std_b = tc.draw(publicly_built_std_votes());

    let _ = std_a.partial_cmp(&std_b);
}

/// `RaftVote::partial_cmp` compares any two vote implementations, and `PartialOrd for Vote` is the
/// same comparison specialized to `Vote`. The two must not drift apart.
#[hegel::test]
fn test_raft_vote_partial_cmp_agrees_with_partial_ord(tc: hegel::TestCase) {
    use crate::vote::RaftVote;

    let std_a = tc.draw(votes::<StdLeaderId>());
    let std_b = tc.draw(votes::<StdLeaderId>());
    assert_eq!(
        PartialOrd::partial_cmp(&std_a, &std_b),
        RaftVote::partial_cmp(&std_a, &std_b),
        "{std_a} vs {std_b}"
    );

    let adv_a = tc.draw(votes::<AdvLeaderId>());
    let adv_b = tc.draw(votes::<AdvLeaderId>());
    assert_eq!(
        PartialOrd::partial_cmp(&adv_a, &adv_b),
        RaftVote::partial_cmp(&adv_a, &adv_b),
        "{adv_a} vs {adv_b}"
    );
}

/// `LeaderId` and `CommittedLeaderId` compare in both directions through two hand-written impls;
/// the reciprocal one must be the mirror image of the other.
#[hegel::test]
fn test_std_leader_id_and_committed_leader_id_compare_symmetrically(tc: hegel::TestCase) {
    let term = tc.draw(term_or_node_id());
    let voted_for = tc.draw(term_or_node_id());
    let committed_term = tc.draw(term_or_node_id());

    let leader_id = StdLeaderId { term, voted_for };
    let committed = leader_id_std::CommittedLeaderId::new(committed_term);

    assert_eq!(
        leader_id.partial_cmp(&committed),
        committed.partial_cmp(&leader_id).map(Ordering::reverse),
        "{leader_id} vs {committed}"
    );
}

/// A vote survives a round trip through a self-describing format and through a tagged one.
/// `LeaderId::voted_for` is serialized as an `Option` to keep the encoding identical to
/// openraft-0.9, which only shows up in a tagged format such as bincode.
#[cfg(feature = "serde")]
#[hegel::test]
fn test_vote_serde_roundtrip(tc: hegel::TestCase) {
    let std_vote = tc.draw(votes::<StdLeaderId>());
    let json = serde_json::to_string(&std_vote).unwrap();
    assert_eq!(std_vote, serde_json::from_str(&json).unwrap());
    let binary = bincode::serialize(&std_vote).unwrap();
    assert_eq!(std_vote, bincode::deserialize(&binary).unwrap());

    let adv_vote = tc.draw(votes::<AdvLeaderId>());
    let json = serde_json::to_string(&adv_vote).unwrap();
    assert_eq!(adv_vote, serde_json::from_str(&json).unwrap());
    let binary = bincode::serialize(&adv_vote).unwrap();
    assert_eq!(adv_vote, bincode::deserialize(&binary).unwrap());
}
