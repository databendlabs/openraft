use std::fmt::Debug;
use std::fmt::Display;

use crate::RaftTypeConfig;
use crate::base::OptionalFeatures;
use crate::vote::leader_id::raft_committed_leader_id::RaftCommittedLeaderId;

/// A Leader identifier in an OpenRaft cluster.
///
/// In OpenRaft, a `LeaderId` represents either:
/// - A granted leader that received votes from a quorum (a `Leader` in standard Raft)
/// - A non-granted leader, i.e., candidate (a `Candidate` in standard Raft)
///
/// The identity of the leader remains the same in both cases. Whether it is granted by a quorum
/// is determined by the `committed` field in [`Vote`].
///
/// # Partial Ordering
///
/// [`RaftLeaderId`] implements `PartialOrd` but not `Ord`. Because to be compatible with standard
/// Raft, in which a `LeaderId` or `CandidateId` is a tuple of `(term, node_id)`: Two such IDs
/// with the same term but different node IDs (e.g., `(1,2)` and `(1,3)`) have no defined ordering -
/// neither can overwrite the other.
///
/// Note: We require `impl PartialOrd<Self::Committed> for Self` but not
/// `impl PartialOrd<Self> for Self::Committed`. This is because in standard Raft,
/// `CommittedLeaderId` is typically just a `u64` term number, and we cannot implement
/// external traits for primitive types.
///
/// [`Vote`]: crate::vote::Vote
pub trait RaftLeaderId<C>
where
    C: RaftTypeConfig,
    Self: OptionalFeatures + PartialOrd + Eq + Clone + Debug + Display + Default + 'static,
    Self: PartialOrd<Self::Committed>,
{
    /// The committed version of this leader ID.
    ///
    /// A simple implementation of this trait would return `Self` as the committed version.
    type Committed: RaftCommittedLeaderId;

    /// Create a new leader ID for the given term and node.
    fn new(term: C::Term, node_id: C::NodeId) -> Self;

    /// Get the term number of this leader
    fn term(&self) -> C::Term;

    /// Get the node ID of this leader if one is set
    fn node_id(&self) -> Option<&C::NodeId>;

    /// Convert this leader ID to a committed leader ID.
    ///
    /// This is used when it has been granted by a quorum.
    fn to_committed(&self) -> Self::Committed;
}

/// Extension methods for [`RaftLeaderId`].
///
/// This trait is implemented for all types that implement [`RaftLeaderId`].
pub trait RaftLeaderIdExt<C>
where
    C: RaftTypeConfig,
    Self: RaftLeaderId<C>,
{
    /// Create a new committed leader ID.
    fn new_committed(term: C::Term, node_id: C::NodeId) -> Self::Committed {
        Self::new(term, node_id).to_committed()
    }

    /// Get the node ID of this leader as an owned value.
    fn to_node_id(&self) -> Option<C::NodeId> {
        self.node_id().cloned()
    }
}

impl<C, T> RaftLeaderIdExt<C> for T
where
    C: RaftTypeConfig,
    T: RaftLeaderId<C>,
{
}
