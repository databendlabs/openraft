//! [`RaftLeaderId`] implementation that enforces standard Raft behavior of at most one leader per
//! term.

use std::cmp::Ordering;
use std::fmt;
use std::ops::Deref;
use std::ops::DerefMut;

use openraft_macros::since;

use crate::NodeId;
use crate::vote::LeaderIdCompare;
use crate::vote::RaftLeaderId;
use crate::vote::RaftTerm;

/// ID of a `leader`, enforcing a single leader per term.
///
/// It includes the `term` and the `node_id`.
///
/// Raft specifies that in a term there is at most one leader, thus Leader ID is partially order as
/// defined below.
#[since(version = "0.10.0", change = "`voted_for` from `Option<NID>` to `NID`")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct LeaderId<Term, NID>
where
    Term: RaftTerm,
    NID: NodeId,
{
    /// The term of the leader.
    pub term: Term,

    /// The node ID that was voted for in this term.
    ///
    /// A `LeaderId` always votes for a node, thus this field is not an `Option`. It is still
    /// serialized as `Option<NID>`, to keep the encoding identical to openraft-0.9 and earlier, in
    /// which this field was an `Option`. A `None` is rejected when decoding.
    #[cfg_attr(feature = "serde", serde(with = "serde_voted_for"))]
    pub voted_for: NID,
}

/// Serialize [`LeaderId::voted_for`] as `Option<NID>`, the encoding used by openraft-0.9 and
/// earlier.
///
/// Self-describing formats such as JSON encode `Some(x)` the same as `x`, but tagged formats such
/// as bincode prefix `Option` with a discriminant byte. Keeping the `Option` in the encoding makes
/// the on-disk data identical in either kind of format, so no data migration is required, and a
/// binary of an earlier version still decodes data written by this version.
#[cfg(feature = "serde")]
mod serde_voted_for {
    use serde::Deserialize;
    use serde::Deserializer;
    use serde::Serialize;
    use serde::Serializer;

    pub(super) fn serialize<S, NID>(voted_for: &NID, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        NID: Serialize,
    {
        Some(voted_for).serialize(serializer)
    }

    pub(super) fn deserialize<'de, D, NID>(deserializer: D) -> Result<NID, D::Error>
    where
        D: Deserializer<'de>,
        NID: Deserialize<'de>,
    {
        Option::<NID>::deserialize(deserializer)?
            .ok_or_else(|| serde::de::Error::custom("LeaderId.voted_for must not be null"))
    }
}

impl<Term, NID> PartialOrd for LeaderId<Term, NID>
where
    Term: RaftTerm,
    NID: NodeId,
{
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        LeaderIdCompare::std(self, other)
    }
}

impl<Term, NID> fmt::Display for LeaderId<Term, NID>
where
    Term: RaftTerm,
    NID: NodeId,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "T{}-N{}", self.term, self.voted_for)
    }
}

impl<Term, NID> PartialEq<CommittedLeaderId<Term>> for LeaderId<Term, NID>
where
    Term: RaftTerm,
    NID: NodeId,
{
    fn eq(&self, _other: &CommittedLeaderId<Term>) -> bool {
        // Committed and non-committed are never equal
        false
    }
}

impl<Term, NID> PartialOrd<CommittedLeaderId<Term>> for LeaderId<Term, NID>
where
    Term: RaftTerm,
    NID: NodeId,
{
    fn partial_cmp(&self, other: &CommittedLeaderId<Term>) -> Option<Ordering> {
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
impl<Term, NID> PartialEq<LeaderId<Term, NID>> for CommittedLeaderId<Term>
where
    Term: RaftTerm,
    NID: NodeId,
{
    fn eq(&self, _other: &LeaderId<Term, NID>) -> bool {
        false
    }
}

/// Reciprocal comparison: `CommittedLeaderId` compared with `LeaderId`.
///
/// Not required by [`RaftLeaderId`] trait bound, but provides symmetric comparison semantics.
impl<Term, NID> PartialOrd<LeaderId<Term, NID>> for CommittedLeaderId<Term>
where
    Term: RaftTerm,
    NID: NodeId,
{
    fn partial_cmp(&self, other: &LeaderId<Term, NID>) -> Option<Ordering> {
        if self.term == other.term {
            Some(Ordering::Greater)
        } else {
            self.term.partial_cmp(&other.term)
        }
    }
}

impl<Term, NID> RaftLeaderId for LeaderId<Term, NID>
where
    Term: RaftTerm,
    NID: NodeId,
{
    type Term = Term;
    type NodeId = NID;
    type Committed = CommittedLeaderId<Term>;

    fn new(term: Term, node_id: NID) -> Self {
        Self {
            term,
            voted_for: node_id,
        }
    }

    fn term(&self) -> Term {
        self.term
    }

    fn node_id(&self) -> &NID {
        &self.voted_for
    }

    fn to_committed(&self) -> Self::Committed {
        CommittedLeaderId::new(self.term)
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
/// In std mode, `CommittedLeaderId` is just a wrapper of `Term`, which is an integer in most
/// cases (u64, u32, u16, u8, i64, i32, i16, i8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(PartialOrd, Ord)]
#[derive(derive_more::Display)]
#[display("{}", term)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct CommittedLeaderId<Term>
where Term: RaftTerm
{
    /// The term of the committed leader.
    pub term: Term,
}

impl<Term> CommittedLeaderId<Term>
where Term: RaftTerm
{
    /// Create a new committed leader ID for the given term.
    pub fn new(term: Term) -> Self {
        Self { term }
    }
}

impl<Term> Deref for CommittedLeaderId<Term>
where Term: RaftTerm
{
    type Target = Term;

    fn deref(&self) -> &Self::Target {
        &self.term
    }
}

impl<Term> DerefMut for CommittedLeaderId<Term>
where Term: RaftTerm
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.term
    }
}

#[rustfmt::skip]
mod impl_from_int {
    use crate::vote::leader_id_std::CommittedLeaderId;

    impl From<u64> for CommittedLeaderId<u64> {fn from(term: u64) -> Self {Self {term}}}
    impl From<u32> for CommittedLeaderId<u32> {fn from(term: u32) -> Self {Self {term}}}
    impl From<u16> for CommittedLeaderId<u16> {fn from(term: u16) -> Self {Self {term}}}
    impl From<u8>  for CommittedLeaderId<u8>  {fn from(term: u8)  -> Self {Self {term}}}
    impl From<i64> for CommittedLeaderId<i64> {fn from(term: i64) -> Self {Self {term}}}
    impl From<i32> for CommittedLeaderId<i32> {fn from(term: i32) -> Self {Self {term}}}
    impl From<i16> for CommittedLeaderId<i16> {fn from(term: i16) -> Self {Self {term}}}
    impl From<i8>  for CommittedLeaderId<i8>  {fn from(term: i8)  -> Self {Self {term}}}
}

#[cfg(test)]
#[allow(clippy::nonminimal_bool)]
mod tests {
    use super::LeaderId;
    use crate::vote::RaftLeaderId;

    #[cfg(feature = "serde")]
    #[test]
    fn test_committed_leader_id_serde() -> anyhow::Result<()> {
        use super::CommittedLeaderId;

        let c = CommittedLeaderId::<u64>::new(5);
        let s = serde_json::to_string(&c)?;
        assert_eq!(r#"5"#, s);

        let c2: CommittedLeaderId<u64> = serde_json::from_str(&s)?;
        assert_eq!(CommittedLeaderId::new(5), c2);

        Ok(())
    }

    /// `voted_for` must be encoded as `Option<NID>`, the shape used by openraft-0.9 and earlier.
    ///
    /// A self-describing format such as JSON encodes `Some(x)` the same as `x`, but a tagged
    /// format such as bincode prefixes an `Option` with a discriminant byte.
    #[cfg(feature = "serde")]
    #[test]
    fn test_leader_id_serde_compat_with_option() -> anyhow::Result<()> {
        /// `LeaderId` as defined by openraft-0.9 and earlier.
        #[derive(serde::Deserialize, serde::Serialize)]
        struct LegacyLeaderId {
            term: u64,
            voted_for: Option<u64>,
        }

        let legacy = LegacyLeaderId {
            term: 5,
            voted_for: Some(3),
        };
        let lid = LeaderId::<u64, u64>::new(5, 3);

        let legacy_json = serde_json::to_string(&legacy)?;
        assert_eq!(legacy_json, serde_json::to_string(&lid)?);
        let decoded: LeaderId<u64, u64> = serde_json::from_str(&legacy_json)?;
        assert_eq!(lid, decoded);

        let legacy_bin = bincode::serialize(&legacy)?;
        assert_eq!(legacy_bin, bincode::serialize(&lid)?);
        let decoded: LeaderId<u64, u64> = bincode::deserialize(&legacy_bin)?;
        assert_eq!(lid, decoded);

        Ok(())
    }

    /// A `None` written by openraft-0.9 or earlier is rejected when decoding, instead of building a
    /// `LeaderId` without a node id.
    #[cfg(feature = "serde")]
    #[test]
    fn test_leader_id_serde_reject_none() -> anyhow::Result<()> {
        let res = serde_json::from_str::<LeaderId<u64, u64>>(r#"{"term":5,"voted_for":null}"#);
        assert_eq!(
            "LeaderId.voted_for must not be null at line 1 column 27",
            res.unwrap_err().to_string()
        );

        let legacy_none = bincode::serialize(&(5u64, None::<u64>))?;
        let res = bincode::deserialize::<LeaderId<u64, u64>>(&legacy_none);
        assert_eq!("LeaderId.voted_for must not be null", res.unwrap_err().to_string());

        Ok(())
    }

    #[test]
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn test_std_leader_id_partial_order() -> anyhow::Result<()> {
        #[allow(clippy::redundant_closure)]
        let lid = |term, node_id| LeaderId::<u64, u64>::new(term, node_id);

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

        let lid = |term, node_id| LeaderId::<u64, u64>::new(term, node_id);
        let clid = |term| CommittedLeaderId::<u64>::new(term);

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

        let lid = |term, node_id| LeaderId::<u64, u64>::new(term, node_id);
        let clid = |term| CommittedLeaderId::<u64>::new(term);

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

        let clid = CommittedLeaderId::<u64>::new(5);
        assert_eq!(5, *clid);

        let term_ref: &u64 = &clid;
        assert_eq!(&5, term_ref);

        Ok(())
    }

    #[test]
    fn test_committed_leader_id_deref_mut() -> anyhow::Result<()> {
        use super::CommittedLeaderId;

        let mut clid = CommittedLeaderId::<u64>::new(5);
        *clid = 10;
        assert_eq!(10, *clid);
        assert_eq!(CommittedLeaderId::new(10), clid);

        Ok(())
    }

    #[test]
    fn test_committed_leader_id_from_int() -> anyhow::Result<()> {
        use super::CommittedLeaderId;

        macro_rules! test_from {
            ($term_type:ty, $value:expr) => {{
                let clid: CommittedLeaderId<$term_type> = $value.into();
                assert_eq!(CommittedLeaderId::new($value), clid);

                let clid = CommittedLeaderId::<$term_type>::from($value);
                assert_eq!(CommittedLeaderId::new($value), clid);
            }};
        }

        test_from!(u64, 5u64);
        test_from!(u32, 5u32);
        test_from!(u16, 5u16);
        test_from!(u8, 5u8);
        test_from!(i64, 5i64);
        test_from!(i32, 5i32);
        test_from!(i16, 5i16);
        test_from!(i8, 5i8);

        Ok(())
    }
}
