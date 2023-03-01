use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::Node;
use crate::NodeId;

#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum ChangeMembers<NID: NodeId, N: Node> {
    /// Upgrade learners to voters.
    ///
    /// The learners have to present or [`error::LearnerNotFound`](`crate::error::LearnerNotFound`)
    /// error will be returned.
    AddVoterIds(BTreeSet<NID>),

    /// Add voters with corresponding nodes.
    AddVoters(BTreeMap<NID, N>),

    /// Remove voters, leave removed voters as learner or not.
    RemoveVoters(BTreeSet<NID>),

    /// Replace voter ids with a new set. The node of every new voter has to already be a learner.
    ReplaceAllVoters(BTreeSet<NID>),

    /// Add nodes to membership, as learners.
    AddNodes(BTreeMap<NID, N>),

    /// Remove nodes from membership.
    ///
    /// If a node is still a voter, it returns
    /// [`error::LearnerNotFound`](`crate::error::LearnerNotFound`) error.
    RemoveNodes(BTreeSet<NID>),

    /// Replace all nodes with a new set.
    ///
    /// Every voter has to have a corresponding node in the new
    /// set, otherwise it returns [`error::LearnerNotFound`](`crate::error::LearnerNotFound`) error.
    ReplaceAllNodes(BTreeMap<NID, N>),
}

/// Convert a series of ids to a `Replace` operation.
impl<NID, N, I> From<I> for ChangeMembers<NID, N>
where
    NID: NodeId,
    N: Node,
    I: IntoIterator<Item = NID>,
{
    fn from(r: I) -> Self {
        let ids = r.into_iter().collect::<BTreeSet<NID>>();
        ChangeMembers::ReplaceAllVoters(ids)
    }
}
