use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::Node;
use crate::NodeId;

#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum ChangeMembers<NID: NodeId, N: Node> {
    AddVoter(BTreeSet<NID>),
    RemoveVoter(BTreeSet<NID>),
    ReplaceAllVoters(BTreeSet<NID>),
    AddNodes(BTreeMap<NID, N>),
    RemoveNodes(BTreeSet<NID>),
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
