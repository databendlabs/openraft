use std::cmp::Ordering;
use std::fmt;
use std::marker::PhantomData;

use crate::display_ext::DisplayOptionExt;
use crate::vote::RaftCommittedLeaderId;
use crate::vote::RaftLeaderId;
use crate::RaftTypeConfig;

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
        match PartialOrd::partial_cmp(&self.term, &other.term) {
            Some(Ordering::Equal) => {
                //
                match (&self.voted_for, &other.voted_for) {
                    (None, None) => Some(Ordering::Equal),
                    (Some(_), None) => Some(Ordering::Greater),
                    (None, Some(_)) => Some(Ordering::Less),
                    (Some(a), Some(b)) => {
                        if a == b {
                            Some(Ordering::Equal)
                        } else {
                            None
                        }
                    }
                }
            }
            cmp => cmp,
        }
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

    fn node_id_ref(&self) -> Option<&C::NodeId> {
        self.voted_for.as_ref()
    }

    fn to_committed(&self) -> Self::Committed {
        CommittedLeaderId::new(self.term, C::NodeId::default())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct CommittedLeaderId<C>
where C: RaftTypeConfig
{
    pub term: C::Term,
    p: PhantomData<C>,
}

impl<C> fmt::Display for CommittedLeaderId<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.term)
    }
}

impl<C> CommittedLeaderId<C>
where C: RaftTypeConfig
{
    pub fn new(term: C::Term, node_id: C::NodeId) -> Self {
        let _ = node_id;
        Self { term, p: PhantomData }
    }
}

impl<C> RaftCommittedLeaderId<C> for CommittedLeaderId<C> where C: RaftTypeConfig {}

#[cfg(test)]
#[allow(clippy::nonminimal_bool)]
mod tests {
    use crate::engine::testing::UTConfig;
    use crate::LeaderId;

    #[cfg(feature = "serde")]
    #[test]
    fn test_committed_leader_id_serde() -> anyhow::Result<()> {
        use crate::CommittedLeaderId;

        let c = CommittedLeaderId::<UTConfig>::new(5, 10);
        let s = serde_json::to_string(&c)?;
        assert_eq!(r#"5"#, s);

        let c2: CommittedLeaderId<UTConfig> = serde_json::from_str(&s)?;
        assert_eq!(CommittedLeaderId::new(5, 0), c2);

        Ok(())
    }

    #[test]
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn test_leader_id_partial_order() -> anyhow::Result<()> {
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
