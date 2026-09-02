//! Tests for the `Vote` ordering semantics.
//!
//! Vote ordering carries Raft's safety argument: a vote is granted only when it is greater than
//! the stored one, so `PartialOrd` has to be a real partial order. The two leader-id flavors order
//! differently by design. `leader_id_std` leaves same-term different-node leader ids incomparable
//! and `RefVote::partial_cmp` hand-writes how committed votes rank against them, so its votes are
//! checked against the `PartialOrd` laws; the relation itself is pinned by example in
//! `test_vote_partial_order`. `leader_id_adv` is totally ordered, so its votes are pinned to a
//! tuple order, which satisfies the laws by construction.
//!
//! Terms and node ids range over [`POOL`], and each test walks every pair or triple of votes over
//! it. Comparison only tests terms and node ids for equality and order, so three values of each
//! reach every branch.

use std::cmp::Ordering;
use std::ops::Range;

use crate::Vote;
use crate::vote::RaftLeaderId;
use crate::vote::leader_id_adv;
use crate::vote::leader_id_std;

type StdLeaderId = leader_id_std::LeaderId<u64, u64>;
type AdvLeaderId = leader_id_adv::LeaderId<u64, u64>;

/// The terms and node ids the tests enumerate.
const POOL: Range<u64> = 0..3;

/// Every vote over [`POOL`]: each term, each node id, committed or not.
fn all_votes<LID>() -> Vec<Vote<LID>>
where LID: RaftLeaderId<Term = u64, NodeId = u64> {
    let mut votes = Vec::new();
    for term in POOL {
        for node_id in POOL {
            votes.push(Vote::new(term, node_id));
            votes.push(Vote::new_committed(term, node_id));
        }
    }
    votes
}

/// Comparing two votes the other way round yields the reversed answer, including the incomparable
/// case: `PartialOrd` requires `a < b` iff `b > a`.
#[test]
fn test_std_vote_partial_cmp_reverses_with_its_operands() {
    let votes = all_votes::<StdLeaderId>();
    for a in &votes {
        for b in &votes {
            let reversed = b.partial_cmp(a).map(Ordering::reverse);
            assert_eq!(a.partial_cmp(b), reversed, "{a} vs {b}");
        }
    }
}

/// `partial_cmp` reports `Equal` exactly for the votes `PartialEq` considers equal.
#[test]
fn test_std_vote_partial_cmp_equal_agrees_with_eq() {
    let votes = all_votes::<StdLeaderId>();
    for a in &votes {
        for b in &votes {
            let is_equal = a.partial_cmp(b) == Some(Ordering::Equal);
            assert_eq!(a == b, is_equal, "{a} vs {b}");
        }
    }
}

/// `a <= b` and `b <= c` imply `a <= c`, which is what lets vote-granting decisions chain. The
/// "committed wins between incomparable leader ids" rule in `RefVote::partial_cmp` is where it
/// could break.
#[test]
fn test_std_vote_partial_cmp_is_transitive() {
    let votes = all_votes::<StdLeaderId>();
    for a in &votes {
        for b in &votes {
            for c in &votes {
                if a <= b && b <= c {
                    assert!(a <= c, "{a} <= {b} <= {c} but not {a} <= {c}");
                }
            }
        }
    }
}

/// With `leader_id_adv`, votes order lexicographically by term, then node id, then commit status,
/// so no pair is incomparable.
#[test]
fn test_adv_vote_order_matches_the_tuple_oracle() {
    let key = |v: &Vote<AdvLeaderId>| (v.leader_id.term, v.leader_id.node_id, v.committed);

    let votes = all_votes::<AdvLeaderId>();
    for a in &votes {
        for b in &votes {
            assert_eq!(Some(key(a).cmp(&key(b))), a.partial_cmp(b), "{a} vs {b}");
        }
    }
}
