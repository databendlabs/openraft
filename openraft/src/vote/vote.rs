use std::cmp::Ordering;
use std::fmt::Formatter;

use crate::vote::committed::CommittedVote;
use crate::vote::leader_id::CommittedLeaderId;
use crate::vote::ref_vote::RefVote;
use crate::vote::vote_status::VoteStatus;
use crate::vote::NonCommittedVote;
use crate::LeaderId;
use crate::RaftTypeConfig;

/// `Vote` represent the privilege of a node.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Vote<C: RaftTypeConfig> {
    /// The id of the node that tries to become the leader.
    pub leader_id: LeaderId<C>,

    pub committed: bool,
}

impl<C> Copy for Vote<C>
where
    C: RaftTypeConfig,
    C::NodeId: Copy,
{
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
        write!(
            f,
            "<{}:{}>",
            self.leader_id,
            if self.is_committed() { "Q" } else { "-" }
        )
    }
}

impl<C> Vote<C>
where C: RaftTypeConfig
{
    pub fn new(term: u64, node_id: C::NodeId) -> Self {
        Self {
            leader_id: LeaderId::new(term, node_id),
            committed: false,
        }
    }

    pub fn new_committed(term: u64, node_id: C::NodeId) -> Self {
        Self {
            leader_id: LeaderId::new(term, node_id),
            committed: true,
        }
    }

    #[deprecated(note = "use `into_committed()` instead", since = "0.10.0")]
    pub fn commit(&mut self) {
        self.committed = true
    }

    pub(crate) fn as_ref_vote(&self) -> RefVote<'_, C> {
        RefVote::new(&self.leader_id, self.committed)
    }

    /// Convert this vote into a `CommittedVote`
    pub(crate) fn into_committed(self) -> CommittedVote<C> {
        CommittedVote::new(self)
    }

    pub(crate) fn into_non_committed(self) -> NonCommittedVote<C> {
        NonCommittedVote::new(self)
    }

    pub(crate) fn into_vote_status(self) -> VoteStatus<C> {
        if self.committed {
            VoteStatus::Committed(self.into_committed())
        } else {
            VoteStatus::Pending(self.into_non_committed())
        }
    }

    pub fn is_committed(&self) -> bool {
        self.committed
    }

    /// Return the [`LeaderId`] this vote represents for.
    ///
    /// The leader may or may not be granted by a quorum.
    pub fn leader_id(&self) -> &LeaderId<C> {
        &self.leader_id
    }

    pub(crate) fn is_same_leader(&self, leader_id: &CommittedLeaderId<C>) -> bool {
        self.leader_id().is_same_as_committed(leader_id)
    }
}

#[cfg(test)]
#[allow(clippy::nonminimal_bool)]
mod tests {
    #[cfg(not(feature = "single-term-leader"))]
    mod feature_no_single_term_leader {
        use crate::engine::testing::UTConfig;
        use crate::Vote;

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

    #[cfg(feature = "single-term-leader")]
    mod feature_single_term_leader {
        use std::panic::UnwindSafe;

        use crate::engine::testing::UTConfig;
        use crate::LeaderId;
        use crate::Vote;

        #[cfg(feature = "serde")]
        #[test]
        fn test_vote_serde() -> anyhow::Result<()> {
            let v = Vote::new(1, 2);
            let s = serde_json::to_string(&v)?;
            assert_eq!(r#"{"leader_id":{"term":1,"voted_for":2},"committed":false}"#, s);

            let v2: Vote<UTConfig> = serde_json::from_str(&s)?;
            assert_eq!(v, v2);

            Ok(())
        }

        #[test]
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        fn test_vote_partial_order() -> anyhow::Result<()> {
            #[allow(clippy::redundant_closure)]
            let vote = |term, node_id| Vote::<UTConfig>::new(term, node_id);

            let none = |term| Vote::<UTConfig> {
                leader_id: LeaderId { term, voted_for: None },
                committed: false,
            };

            #[allow(clippy::redundant_closure)]
            let committed = |term, node_id| Vote::<UTConfig>::new_committed(term, node_id);

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
