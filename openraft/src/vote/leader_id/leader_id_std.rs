use std::cmp::Ordering;
use std::fmt;
use std::marker::PhantomData;

use crate::display_ext::DisplayOptionExt;
use crate::RaftTypeConfig;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct LeaderId<C>
where C: RaftTypeConfig
{
    pub term: u64,

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

impl<C> LeaderId<C>
where C: RaftTypeConfig
{
    pub fn new(term: u64, node_id: C::NodeId) -> Self {
        Self {
            term,
            voted_for: Some(node_id),
        }
    }

    pub fn get_term(&self) -> u64 {
        self.term
    }

    pub fn voted_for(&self) -> Option<C::NodeId> {
        self.voted_for.clone()
    }

    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_committed(&self) -> CommittedLeaderId<C> {
        CommittedLeaderId::new(self.term, C::NodeId::default())
    }

    /// Return if it is the same leader as the committed leader id.
    ///
    /// A committed leader may have less info than a non-committed.
    pub(crate) fn is_same_as_committed(&self, other: &CommittedLeaderId<C>) -> bool {
        self.term == other.term
    }
}

impl<C> fmt::Display for LeaderId<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "T{}-N{}", self.term, self.voted_for.display())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct CommittedLeaderId<C> {
    pub term: u64,
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
    pub fn new(term: u64, node_id: C::NodeId) -> Self {
        let _ = node_id;
        Self { term, p: PhantomData }
    }
}

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
