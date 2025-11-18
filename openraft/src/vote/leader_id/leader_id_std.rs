//! [`RaftLeaderId`] implementation that enforces standard Raft behavior of at most one leader per
//! term.

use std::cmp::Ordering;
use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ops::DerefMut;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::vote::LeaderIdCompare;
use crate::vote::RaftLeaderId;

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
    /// The term of the leader.
    pub term: C::Term,

    /// The node ID that was voted for in this term.
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

impl<C> PartialEq<CommittedLeaderId<C>> for LeaderId<C>
where C: RaftTypeConfig
{
    fn eq(&self, _other: &CommittedLeaderId<C>) -> bool {
        // Committed and non-committed are never equal
        false
    }
}

impl<C> PartialOrd<CommittedLeaderId<C>> for LeaderId<C>
where C: RaftTypeConfig
{
    fn partial_cmp(&self, other: &CommittedLeaderId<C>) -> Option<Ordering> {
        if self.term == other.term {
            // For the same term, committed Leader overrides non-committed
            Some(Ordering::Less)
        } else {
            self.term.partial_cmp(&other.term)
        }
    }
}

/// Reciprocal comparison: `CommittedLeaderId` compared with `LeaderId`.
///
/// Not required by [`RaftLeaderId`] trait bound, but provides symmetric comparison semantics.
impl<C> PartialEq<LeaderId<C>> for CommittedLeaderId<C>
where C: RaftTypeConfig
{
    fn eq(&self, _other: &LeaderId<C>) -> bool {
        false
    }
}

/// Reciprocal comparison: `CommittedLeaderId` compared with `LeaderId`.
///
/// Not required by [`RaftLeaderId`] trait bound, but provides symmetric comparison semantics.
impl<C> PartialOrd<LeaderId<C>> for CommittedLeaderId<C>
where C: RaftTypeConfig
{
    fn partial_cmp(&self, other: &LeaderId<C>) -> Option<Ordering> {
        if self.term == other.term {
            Some(Ordering::Greater)
        } else {
            self.term.partial_cmp(&other.term)
        }
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

    fn node_id(&self) -> &C::NodeId {
        self.voted_for.as_ref().unwrap()
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
///
/// In std mode, `CommittedLeaderId` is just a wrapper of `C::Term`, which is an integer in most
/// cases (u64, u32, u16, u8, i64, i32, i16, i8).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(PartialOrd, Ord)]
#[derive(derive_more::Display)]
#[display("{}", term)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct CommittedLeaderId<C>
where C: RaftTypeConfig
{
    /// The term of the committed leader.
    pub term: C::Term,
    p: PhantomData<C>,
}

impl<C> CommittedLeaderId<C>
where C: RaftTypeConfig
{
    /// Create a new committed leader ID for the given term.
    pub fn new(term: C::Term, node_id: C::NodeId) -> Self {
        let _ = node_id;
        Self { term, p: PhantomData }
    }
}

impl<C> Deref for CommittedLeaderId<C>
where C: RaftTypeConfig
{
    type Target = C::Term;

    fn deref(&self) -> &Self::Target {
        &self.term
    }
}

impl<C> DerefMut for CommittedLeaderId<C>
where C: RaftTypeConfig
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.term
    }
}

#[rustfmt::skip]
mod impl_from_int {
    use crate::RaftTypeConfig;
    use crate::vote::leader_id_std::CommittedLeaderId;

    impl<C: RaftTypeConfig<Term=u64>> From<u64> for CommittedLeaderId<C> {fn from(term: u64) -> Self {Self {term,..Default::default()}}}
    impl<C: RaftTypeConfig<Term=u32>> From<u32> for CommittedLeaderId<C> {fn from(term: u32) -> Self {Self {term,..Default::default()}}}
    impl<C: RaftTypeConfig<Term=u16>> From<u16> for CommittedLeaderId<C> {fn from(term: u16) -> Self {Self {term,..Default::default()}}}
    impl<C: RaftTypeConfig<Term=u8>>  From<u8>  for CommittedLeaderId<C> {fn from(term: u8)  -> Self {Self {term,..Default::default()}}}
    impl<C: RaftTypeConfig<Term=i64>> From<i64> for CommittedLeaderId<C> {fn from(term: i64) -> Self {Self {term,..Default::default()}}}
    impl<C: RaftTypeConfig<Term=i32>> From<i32> for CommittedLeaderId<C> {fn from(term: i32) -> Self {Self {term,..Default::default()}}}
    impl<C: RaftTypeConfig<Term=i16>> From<i16> for CommittedLeaderId<C> {fn from(term: i16) -> Self {Self {term,..Default::default()}}}
    impl<C: RaftTypeConfig<Term=i8>>  From<i8>  for CommittedLeaderId<C> {fn from(term: i8)  -> Self {Self {term,..Default::default()}}}
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

        // Compare term first
        assert!(lid(2, 2) > lid(1, 2));
        assert!(lid(1, 2) < lid(2, 2));

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

    #[test]
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn test_leader_id_vs_committed_partial_order() -> anyhow::Result<()> {
        use super::CommittedLeaderId;

        let lid = |term, node_id| LeaderId::<UTConfig>::new(term, node_id);
        let clid = |term| CommittedLeaderId::<UTConfig>::new(term, 0);

        // PartialEq: LeaderId and CommittedLeaderId are never equal
        assert!(lid(2, 2) != clid(2));

        // Same term: CommittedLeaderId > LeaderId
        assert!(lid(2, 2) < clid(2));
        assert!(!(lid(2, 2) > clid(2)));

        // Different terms: compare by term
        assert!(lid(1, 2) < clid(2));
        assert!(lid(3, 2) > clid(2));

        Ok(())
    }

    #[test]
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn test_committed_vs_leader_id_partial_order() -> anyhow::Result<()> {
        use super::CommittedLeaderId;

        let lid = |term, node_id| LeaderId::<UTConfig>::new(term, node_id);
        let clid = |term| CommittedLeaderId::<UTConfig>::new(term, 0);

        // PartialEq: CommittedLeaderId and LeaderId are never equal (symmetric)
        assert!(clid(2) != lid(2, 2));
        assert!(clid(2) != lid(2, 5));

        // Same term: CommittedLeaderId > LeaderId (symmetric)
        assert!(clid(2) > lid(2, 2));
        assert!(!(clid(2) < lid(2, 2)));
        assert!(!(clid(2) == lid(2, 2)));

        // Different terms: compare by term (symmetric)
        assert!(clid(2) > lid(1, 2));
        assert!(clid(2) < lid(3, 2));
        assert!(!(clid(2) > lid(3, 2)));
        assert!(!(clid(2) == lid(1, 2)));

        Ok(())
    }

    #[test]
    fn test_committed_leader_id_deref() -> anyhow::Result<()> {
        use super::CommittedLeaderId;

        let clid = CommittedLeaderId::<UTConfig>::new(5, 10);
        assert_eq!(5, *clid);

        let term_ref: &u64 = &clid;
        assert_eq!(&5, term_ref);

        Ok(())
    }

    #[test]
    fn test_committed_leader_id_deref_mut() -> anyhow::Result<()> {
        use super::CommittedLeaderId;

        let mut clid = CommittedLeaderId::<UTConfig>::new(5, 10);
        *clid = 10;
        assert_eq!(10, *clid);
        assert_eq!(CommittedLeaderId::new(10, 10), clid);

        Ok(())
    }

    #[test]
    fn test_committed_leader_id_from_int() -> anyhow::Result<()> {
        use super::CommittedLeaderId;

        macro_rules! test_from {
            ($config:ident, $term_type:ty, $value:expr) => {{
                crate::declare_raft_types!($config: D=u64, R=(), LeaderId=super::LeaderId<$config>, Term=$term_type);

                let clid: CommittedLeaderId<$config> = $value.into();
                assert_eq!(CommittedLeaderId::new($value, 0), clid);

                let clid = CommittedLeaderId::<$config>::from($value);
                assert_eq!(CommittedLeaderId::new($value, 0), clid);
            }};
        }

        test_from!(TcU64, u64, 5u64);
        test_from!(TcU32, u32, 5u32);
        test_from!(TcU16, u16, 5u16);
        test_from!(TcU8, u8, 5u8);
        test_from!(TcI64, i64, 5i64);
        test_from!(TcI32, i32, 5i32);
        test_from!(TcI16, i16, 5i16);
        test_from!(TcI8, i8, 5i8);

        Ok(())
    }
}
