use openraft_macros::since;

use crate::node::NodeId;

/// A write voter of a membership does not have [`NodeRole::ReceiveLogs`].
///
/// OpenRaft replicates logs only to nodes with [`NodeRole::ReceiveLogs`], so such a voter never
/// acknowledges a log, and a write quorum that needs it never commits. OpenRaft rejects each
/// membership that an application supplies with such a voter.
///
/// [`NodeRole::ReceiveLogs`]: crate::membership::NodeRole::ReceiveLogs
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("write voter {node_id} does not have NodeRole::ReceiveLogs")]
pub struct WriteVoterWithoutReceiveLogs<NID>
where NID: NodeId
{
    /// The first write voter without `NodeRole::ReceiveLogs`.
    pub node_id: NID,
}
