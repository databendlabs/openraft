use std::collections::BTreeSet;

use crate::NodeId;

#[derive(PartialEq, Eq, Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum ChangeMembers<NID: NodeId> {
    Add(BTreeSet<NID>),
    Remove(BTreeSet<NID>),
    Replace(BTreeSet<NID>),
}

/// Convert a series of ids to a `Replace` operation.
impl<NID, I> From<I> for ChangeMembers<NID>
where
    NID: NodeId,
    I: IntoIterator<Item = NID>,
{
    fn from(r: I) -> Self {
        let ids = r.into_iter().collect::<BTreeSet<NID>>();
        ChangeMembers::Replace(ids)
    }
}

impl<NID: NodeId> ChangeMembers<NID> {
    /// Apply the `ChangeMembers` to `old` node set, return new node set
    pub fn apply_to(self, old: &BTreeSet<NID>) -> BTreeSet<NID> {
        match self {
            ChangeMembers::Replace(c) => c,
            ChangeMembers::Add(add_members) => old.union(&add_members).cloned().collect::<BTreeSet<_>>(),
            ChangeMembers::Remove(remove_members) => old.difference(&remove_members).cloned().collect::<BTreeSet<_>>(),
        }
    }
}
