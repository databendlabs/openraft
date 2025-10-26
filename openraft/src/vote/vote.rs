use std::cmp::Ordering;
use std::fmt::Formatter;

use crate::RaftTypeConfig;
use crate::vote::RaftLeaderId;
use crate::vote::RaftVote;
use crate::vote::raft_vote::RaftVoteExt;

/// `Vote` represent the privilege of a node.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Vote<C: RaftTypeConfig> {
    /// The id of the node that tries to become the leader.
    pub leader_id: C::LeaderId,

    /// Whether this vote has been committed (granted by a quorum).
    pub committed: bool,
}

impl<C> PartialOrd for Vote<C>
where C: RaftTypeConfig
{
    #[inline]
    fn partial_cmp(&self, other: &Vote<C>) -> Option<Ordering> {
        PartialOrd::partial_cmp(&self.as_ref_vote(), &other.as_ref_vote())
    }
}

impl<C> std::fmt::Display for Vote<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.as_ref_vote().fmt(f)
    }
}

impl<C> RaftVote<C> for Vote<C>
where C: RaftTypeConfig
{
    fn from_leader_id(leader_id: C::LeaderId, committed: bool) -> Self {
        Self { leader_id, committed }
    }

    fn leader_id(&self) -> Option<&C::LeaderId> {
        Some(&self.leader_id)
    }

    fn is_committed(&self) -> bool {
        self.committed
    }
}

impl<C> Vote<C>
where C: RaftTypeConfig
{
    /// Create a new uncommitted vote for the given term and node.
    pub fn new(term: C::Term, node_id: C::NodeId) -> Self {
        Self {
            leader_id: C::LeaderId::new(term, node_id),
            committed: false,
        }
    }

    /// Create a new committed vote for the given term and node.
    pub fn new_committed(term: C::Term, node_id: C::NodeId) -> Self {
        Self {
            leader_id: C::LeaderId::new(term, node_id),
            committed: true,
        }
    }

    /// Mark this vote as committed (deprecated).
    #[deprecated(note = "use `into_committed()` instead", since = "0.10.0")]
    pub fn commit(&mut self) {
        self.committed = true
    }

    /// Check if this vote has been committed.
    pub fn is_committed(&self) -> bool {
        self.committed
    }

    /// Return the `LeaderId` this vote represents for.
    ///
    /// The leader may or may not be granted by a quorum.
    pub fn leader_id(&self) -> &C::LeaderId {
        &self.leader_id
    }
}

#[cfg(test)]
#[allow(clippy::nonminimal_bool)]
mod tests {
    mod feature_no_single_term_leader {
        use crate::Vote;
        use crate::engine::testing::UTConfig;

        #[cfg(feature = "serde")]
        #[test]
        fn test_vote_serde() -> anyhow::Result<()> {
            let v = Vote::new(1, 2);
            let s = serde_json::to_string(&v)?;
            assert_eq!(r#"{"leader_id":{"term":1,"node_id":2},"committed":false}"#, s);

            let v2: Vote<UTConfig> = serde_json::from_str(&s)?;
            assert_eq!(v, v2);

            Ok(())
        }

        #[test]
        fn test_vote_total_order() -> anyhow::Result<()> {
            #[allow(clippy::redundant_closure)]
            let vote = |term, node_id| Vote::<UTConfig>::new(term, node_id);

            #[allow(clippy::redundant_closure)]
            let committed = |term, node_id| Vote::<UTConfig>::new_committed(term, node_id);

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

    mod feature_single_term_leader {
        use std::panic::UnwindSafe;

        use crate::Vote;
        use crate::declare_raft_types;
        use crate::vote::leader_id_std::LeaderId;

        declare_raft_types!(TC: D=u64,R=(),LeaderId=LeaderId<TC>);

        #[cfg(feature = "serde")]
        #[test]
        fn test_vote_serde() -> anyhow::Result<()> {
            let v = Vote::new(1, 2);
            let s = serde_json::to_string(&v)?;
            assert_eq!(r#"{"leader_id":{"term":1,"voted_for":2},"committed":false}"#, s);

            let v2: Vote<TC> = serde_json::from_str(&s)?;
            assert_eq!(v, v2);

            Ok(())
        }

        #[test]
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        fn test_vote_partial_order() -> anyhow::Result<()> {
            #[allow(clippy::redundant_closure)]
            let vote = |term, node_id| Vote::<TC>::new(term, node_id);

            let none = |term| Vote::<TC> {
                leader_id: LeaderId { term, voted_for: None },
                committed: false,
            };

            #[allow(clippy::redundant_closure)]
            let committed = |term, node_id| Vote::<TC>::new_committed(term, node_id);

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
                assert_panic(|| committed(2, 2) > committed(2, 3));
                assert_panic(|| committed(2, 2) >= committed(2, 3));
                assert_panic(|| committed(2, 2) < committed(2, 3));
                assert_panic(|| committed(2, 2) <= committed(2, 3));
            }

            Ok(())
        }
    }
}
