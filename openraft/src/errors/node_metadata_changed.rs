use openraft_macros::since;

use crate::node::NodeId;

/// The proposed membership changes the `Node` of a node id that the cluster already knows.
///
/// A direct membership append refuses to update node metadata, because replacing the address of a
/// node that is already a voter can split the cluster into two groups that talk to different
/// machines under one node id. Adding a new node id and removing an existing one stay allowed.
///
/// An intentional metadata update remains a separate [`ChangeMembers::SetNodes`] operation.
///
/// [`ChangeMembers::SetNodes`]: crate::ChangeMembers::SetNodes
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("proposed membership changes the metadata of existing node {node_id}")]
pub struct NodeMetadataChanged<NID>
where NID: NodeId
{
    /// The node id whose proposed `Node` differs from the one in the current membership.
    pub node_id: NID,
}
