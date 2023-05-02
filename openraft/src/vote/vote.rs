use std::cmp::Ordering;
use std::fmt::Formatter;

use crate::vote::leader_id::CommittedLeaderId;
use crate::LeaderId;
use crate::MessageSummary;
use crate::NodeId;

/// `Vote` represent the privilege of a node.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Vote<NID: NodeId> {
    /// The id of the node that tries to become the leader.
    pub leader_id: LeaderId<NID>,

    pub committed: bool,
}

// Commit vote have a total order relation with all other votes
impl<NID: NodeId> PartialOrd for Vote<NID> {
    #[inline]
    fn partial_cmp(&self, other: &Vote<NID>) -> Option<Ordering> {
        match PartialOrd::partial_cmp(&self.leader_id, &other.leader_id) {
            Some(Ordering::Equal) => PartialOrd::partial_cmp(&self.committed, &other.committed),
            None => {
                // If two leader_id are not comparable, they won't both be granted(committed).
                // Therefore use `committed` to determine greatness to minimize election conflict.
                match (self.committed, other.committed) {
                    (false, false) => None,
                    (true, false) => Some(Ordering::Greater),
                    (false, true) => Some(Ordering::Less),
                    (true, true) => {
                        unreachable!("two incomparable leaders can not be both committed: {} {}", self, other)
                    }
                }
            }
            cmp => cmp,
        }
    }
}

impl<NID: NodeId> std::fmt::Display for Vote<NID> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}",
            self.leader_id,
            if self.is_committed() {
                "committed"
            } else {
                "uncommitted"
            }
        )
    }
}

impl<NID: NodeId> MessageSummary<Vote<NID>> for Vote<NID> {
    fn summary(&self) -> String {
        format!("{}", self)
    }
}

impl<NID: NodeId> Vote<NID> {
    pub fn new(term: u64, node_id: NID) -> Self {
        Self {
            leader_id: LeaderId::new(term, node_id),
            committed: false,
        }
    }
    pub fn new_committed(term: u64, node_id: NID) -> Self {
        Self {
            leader_id: LeaderId::new(term, node_id),
            committed: true,
        }
    }

    pub fn commit(&mut self) {
        self.committed = true
    }

    pub fn is_committed(&self) -> bool {
        self.committed
    }

    /// Return the [`LeaderId`] this vote represents for.
    ///
    /// The leader may or may not be granted by a quorum.
    pub fn leader_id(&self) -> &LeaderId<NID> {
        &self.leader_id
    }

    /// Return a [`CommittedLeaderId`], which is granted by a quorum.
    pub(crate) fn committed_leader_id(&self) -> Option<CommittedLeaderId<NID>> {
        // Special case (term==0): when initializing the first log does not need vote to be committed.
        if self.is_committed() || self.leader_id().term == 0 {
            Some(self.leader_id().to_committed())
        } else {
            None
        }
    }

    pub(crate) fn is_same_leader(&self, leader_id: &CommittedLeaderId<NID>) -> bool {
        self.leader_id().is_same_as_committed(leader_id)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "single-term-leader"))]
    mod feature_no_single_term_leader {
        use crate::Vote;

        #[cfg(feature = "serde")]
        #[test]
        fn test_vote_serde() -> anyhow::Result<()> {
            let v = Vote::new(1, 2);
            let s = serde_json::to_string(&v)?;
            assert_eq!(r#"{"leader_id":{"term":1,"node_id":2},"committed":false}"#, s);

            let v2: Vote<u64> = serde_json::from_str(&s)?;
            assert_eq!(v, v2);

            Ok(())
        }

        #[test]
        fn test_vote_total_order() -> anyhow::Result<()> {
            #[allow(clippy::redundant_closure)]
            let vote = |term, node_id| Vote::<u64>::new(term, node_id);

            #[allow(clippy::redundant_closure)]
            let committed = |term, node_id| Vote::<u64>::new_committed(term, node_id);

            // Compare term first
            assert!(vote(2, 2) > vote(1, 2));
            assert!(vote(1, 2) < vote(2, 2));

            // Equal term
            assert!(vote(2, 2) > vote(2, 1));
            assert!(vote(2, 1) < vote(2, 2));

            // Equal term, node_id
            assert!(vote(2, 2) == vote(2, 2));
            assert!(vote(2, 2) >= vote(2, 2));
            assert!(vote(2, 2) <= vote(2, 2));

            assert!(committed(2, 2) > vote(2, 2));
            assert!(vote(2, 2) < committed(2, 2));
            Ok(())
        }
    }

    #[cfg(feature = "single-term-leader")]
    mod feature_single_term_leader {
        use std::panic::UnwindSafe;

        use crate::LeaderId;
        use crate::Vote;

        #[cfg(feature = "serde")]
        #[test]
        fn test_vote_serde() -> anyhow::Result<()> {
            let v = Vote::new(1, 2);
            let s = serde_json::to_string(&v)?;
            assert_eq!(r#"{"leader_id":{"term":1,"voted_for":2},"committed":false}"#, s);

            let v2: Vote<u64> = serde_json::from_str(&s)?;
            assert_eq!(v, v2);

            Ok(())
        }

        #[test]
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        fn test_vote_partial_order() -> anyhow::Result<()> {
            #[allow(clippy::redundant_closure)]
            let vote = |term, node_id| Vote::<u64>::new(term, node_id);

            let none = |term| Vote::<u64> {
                leader_id: LeaderId { term, voted_for: None },
                committed: false,
            };

            #[allow(clippy::redundant_closure)]
            let committed = |term, node_id| Vote::<u64>::new_committed(term, node_id);

            // Compare term first
            assert!(vote(2, 2) > vote(1, 2));
            assert!(vote(2, 2) >= vote(1, 2));
            assert!(vote(1, 2) < vote(2, 2));
            assert!(vote(1, 2) <= vote(2, 2));

            // Equal term, Some > None
            assert!(vote(2, 2) > none(2));
            assert!(vote(2, 2) >= none(2));
            assert!(none(2) < vote(2, 2));
            assert!(none(2) <= vote(2, 2));

            // Committed greater than non-committed if leader_id is incomparable
            assert!(committed(2, 2) > vote(2, 2));
            assert!(committed(2, 2) >= vote(2, 2));
            assert!(committed(2, 1) > vote(2, 2));
            assert!(committed(2, 1) >= vote(2, 2));

            // Lower term committed is not greater
            assert!(!(committed(1, 1) > vote(2, 1)));
            assert!(!(committed(1, 1) >= vote(2, 1)));

            // Compare to itself
            assert!(committed(1, 1) >= committed(1, 1));
            assert!(committed(1, 1) <= committed(1, 1));
            assert!(committed(1, 1) == committed(1, 1));

            // Equal
            assert!(vote(2, 2) == vote(2, 2));
            assert!(vote(2, 2) >= vote(2, 2));
            assert!(vote(2, 2) <= vote(2, 2));

            // Incomparable
            assert!(!(vote(2, 2) > vote(2, 3)));
            assert!(!(vote(2, 2) >= vote(2, 3)));
            assert!(!(vote(2, 2) < vote(2, 3)));
            assert!(!(vote(2, 2) <= vote(2, 3)));
            assert!(!(vote(2, 2) == vote(2, 3)));

            // Incomparable committed
            {
                fn assert_panic<T, F: FnOnce() -> T + UnwindSafe>(f: F) {
                    let res = std::panic::catch_unwind(f);
                    assert!(res.is_err());
                }
                assert_panic(|| (committed(2, 2) > committed(2, 3)));
                assert_panic(|| (committed(2, 2) >= committed(2, 3)));
                assert_panic(|| (committed(2, 2) < committed(2, 3)));
                assert_panic(|| (committed(2, 2) <= committed(2, 3)));
            }

            Ok(())
        }
    }
}
