use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft_macros::since;

/// The weights or the write weight of a [`WeightedMembership`] config, or the threshold of a
/// [`WeightedQuorumSet`], do not form usable rules.
///
/// [`WeightedQuorumSet::new()`] returns it, and [`WeightedMembership`] and its edits return it in
/// [`MembershipError::UserDefined`]. OpenRaft wraps it with the configured error source when
/// applying an edit inside the Raft core.
///
/// [`WeightedMembership`]: crate::WeightedMembership
/// [`WeightedQuorumSet`]: crate::quorum::WeightedQuorumSet
/// [`WeightedQuorumSet::new()`]: crate::quorum::WeightedQuorumSet::new
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidWeights<NID>
where NID: NodeId
{
    /// A voter has weight 0, so it would be a voter whose vote never counts.
    #[error("voter {node_id} has weight 0; every weight must be positive")]
    ZeroWeight {
        /// The first voter with weight 0.
        node_id: NID,
    },

    /// The weights sum to more than `u64::MAX`.
    #[error("the weights sum to more than u64::MAX")]
    TotalWeightOverflow,

    /// The write weight is 0, so the empty set would be a write quorum, or it exceeds the total
    /// weight, so no set would be one.
    #[error(
        "invalid write weight: write_weight={write_weight}, total_weight={total_weight}; \
         need 1 <= write_weight <= total_weight"
    )]
    WriteWeightOutOfRange {
        /// The weight that a write quorum of the config must reach.
        write_weight: u64,

        /// The sum of the weights of the config.
        total_weight: u64,
    },

    /// The threshold of a [`WeightedQuorumSet`] is 0, so the empty set would be a quorum.
    ///
    /// [`WeightedQuorumSet`]: crate::quorum::WeightedQuorumSet
    #[error("the threshold is 0; it must be positive")]
    ZeroThreshold,
}

impl<NID> From<InvalidWeights<NID>> for MembershipError<NID, InvalidWeights<NID>>
where NID: NodeId
{
    fn from(e: InvalidWeights<NID>) -> Self {
        MembershipError::UserDefined(e)
    }
}
