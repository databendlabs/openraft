//! [`RaftLeaderId`] implementation that enforces standard Raft behavior of at most one leader per
//! term.

use std::cmp::Ordering;
use std::fmt;
use std::marker::PhantomData;

use crate::display_ext::DisplayOptionExt;
use crate::vote::LeaderIdCompare;
use crate::vote::RaftLeaderId;
use crate::RaftTypeConfig;

/// ID of a `leader`, enforcing a single leader per term.
///
/// It includes the `term` and the `node_id`.
///
/// Raft specifies that in a term there is at most one leader, thus Leader ID is partially order as
/// defined below.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct LeaderId<C>
where C: RaftTypeConfig
{
    pub term: C::Term,

    pub voted_for: Option<C::NodeId>,
}

impl<C> PartialOrd for LeaderId<C>
where C: RaftTypeConfig
{
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        LeaderIdCompare::<C>::std(self, other)
    }
}

impl<C> fmt::Display for LeaderId<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "T{}-N{}", self.term, self.voted_for.display())
    }
}

impl<C> RaftLeaderId<C> for LeaderId<C>
where C: RaftTypeConfig
{
    type Committed = CommittedLeaderId<C>;

    fn new(term: C::Term, node_id: C::NodeId) -> Self {
        Self {
            term,
            voted_for: Some(node_id),
        }
    }

    fn term(&self) -> C::Term {
        self.term
    }

    fn node_id(&self) -> Option<&C::NodeId> {
        self.voted_for.as_ref()
    }

    fn to_committed(&self) -> Self::Committed {
        CommittedLeaderId::new(self.term, C::NodeId::default())
    }
}

/// The unique identifier of a leader that is already granted by a quorum in phase-1(voting).
///
/// [`CommittedLeaderId`] contains less information than [`LeaderId`], because it implies the
/// constraint that **a quorum has granted it**.
///
/// For a partial order `LeaderId`, we know that all the committed leader-id must be a total order
/// set. Therefore, once it is granted by a quorum, it only keeps the information that makes
/// leader-ids a correct total order set, e.g., in standard raft, `voted_for: Option<node_id>` can
/// be removed from `(term, voted_for)` once it is granted. This is why standard Raft stores just a
/// `term` in log entry to identify the Leader proposing the log entry.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(PartialOrd, Ord)]
#[derive(derive_more::Display)]
#[display("{}", term)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct CommittedLeaderId<C>
where C: RaftTypeConfig
{
    pub term: C::Term,
    p: PhantomData<C>,
}

impl<C> CommittedLeaderId<C>
where C: RaftTypeConfig
{
    pub fn new(term: C::Term, node_id: C::NodeId) -> Self {
        let _ = node_id;
        Self { term, p: PhantomData }
    }
}

#[cfg(test)]
#[allow(clippy::nonminimal_bool)]
mod tests {
    use super::LeaderId;
    use crate::engine::testing::UTConfig;
    use crate::vote::RaftLeaderId;

    #[cfg(feature = "serde")]
    #[test]
    fn test_committed_leader_id_serde() -> anyhow::Result<()> {
        use super::CommittedLeaderId;

        let c = CommittedLeaderId::<UTConfig>::new(5, 10);
        let s = serde_json::to_string(&c)?;
        assert_eq!(r#"5"#, s);

        let c2: CommittedLeaderId<UTConfig> = serde_json::from_str(&s)?;
        assert_eq!(CommittedLeaderId::new(5, 0), c2);

        Ok(())
    }

    #[test]
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn test_std_leader_id_partial_order() -> anyhow::Result<()> {
        #[allow(clippy::redundant_closure)]
        let lid = |term, node_id| LeaderId::<UTConfig>::new(term, node_id);

        let lid_none = |term| LeaderId::<UTConfig> { term, voted_for: None };

        // Compare term first
        assert!(lid(2, 2) > lid(1, 2));
        assert!(lid(1, 2) < lid(2, 2));

        // Equal term, Some > None
        assert!(lid(2, 2) > lid_none(2));
        assert!(lid_none(2) < lid(2, 2));

        // Equal
        assert!(lid(2, 2) == lid(2, 2));
        assert!(lid(2, 2) >= lid(2, 2));
        assert!(lid(2, 2) <= lid(2, 2));

        // Incomparable
        assert!(!(lid(2, 2) > lid(2, 3)));
        assert!(!(lid(2, 2) < lid(2, 3)));
        assert!(!(lid(2, 2) == lid(2, 3)));

        Ok(())
    }
}
