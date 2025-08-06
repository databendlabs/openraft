use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;

use crate::base::OptionalFeatures;

/// A Leader identifier that has been granted and committed by a quorum of the cluster.
///
/// This type is used as part of the Log ID to identify the leader that proposed a log entry.
/// Because log can only be proposed by a Leader committed by a quorum.
/// For example, in standard Raft, committed LeaderId is just the term number, because in each term
/// there is only one established leader.
///
/// # Implementation
///
/// A simple non-optimized implementation of this trait is to use the same type as [`RaftLeaderId`].
///
/// # Total Ordering
///
/// Unlike [`RaftLeaderId`], this type implements `Ord` because committed leader IDs
/// have a total ordering as they must be agreed upon by a quorum, and two incomparable
/// [`RaftLeaderId`] cannot both be committed by two quorums, because only a **greater**
/// [`RaftLeaderId`] can override an existing value.
///
/// A [`RaftCommittedLeaderId`] may contain less information than the corresponding
/// [`RaftLeaderId`], because it implies the constraint that **a quorum has granted it**.
///
/// For a total order [`RaftLeaderId`], the [`RaftCommittedLeaderId`] is the same.
///
/// For a partial order [`RaftLeaderId`], we know that all the granted leader-id must be a total
/// order set. Therefor once it is granted by a quorum, it only keeps the information that makes
/// leader-ids a correct total order set
///
/// For example, in standard Raft:
/// - [`RaftLeaderId`] is `(term, voted_for)` - partially ordered
/// - [`RaftCommittedLeaderId`] is just `term` - totally ordered (The `voted_for` field can be
///   dropped since it's no longer needed for ordering)
///
/// [`RaftLeaderId`]: crate::vote::RaftLeaderId
#[since(version = "0.10.0")]
pub trait RaftCommittedLeaderId
where Self: OptionalFeatures + Ord + Clone + Debug + Display + Default + 'static
{
}

impl<T> RaftCommittedLeaderId for T where T: OptionalFeatures + Ord + Clone + Debug + Display + Default + 'static {}
