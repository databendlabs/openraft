use std::fmt::Debug;

use super::Membership;
use crate::compat::Upgrade;

/// v0.7 compatible EntryPayload.
///
/// To load from either v0.7 or the latest format data and upgrade it to the latest type:
/// ```ignore
/// let x:openraft::EntryPayload = serde_json::from_slice::<compat07::EntryPayload>(&serialized_bytes)?.upgrade()
/// ```
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum EntryPayload<C: crate::RaftTypeConfig> {
    Blank,
    Normal(C::D),
    Membership(Membership),
}

impl<C> Upgrade<crate::EntryPayload<C>> for or07::EntryPayload<C::D>
where
    C: crate::RaftTypeConfig<NodeId = u64, Node = crate::EmptyNode>,
    <C as crate::RaftTypeConfig>::D: or07::AppData + Debug,
{
    fn upgrade(self) -> crate::EntryPayload<C> {
        match self {
            Self::Blank => crate::EntryPayload::Blank,
            Self::Membership(m) => crate::EntryPayload::Membership(m.upgrade()),
            Self::Normal(d) => crate::EntryPayload::Normal(d),
        }
    }
}

impl<C: crate::RaftTypeConfig<NodeId = u64, Node = crate::EmptyNode>> Upgrade<crate::EntryPayload<C>>
    for EntryPayload<C>
{
    fn upgrade(self) -> crate::EntryPayload<C> {
        match self {
            EntryPayload::Blank => crate::EntryPayload::Blank,
            EntryPayload::Normal(d) => crate::EntryPayload::Normal(d),
            EntryPayload::Membership(m) => crate::EntryPayload::Membership(m.upgrade()),
        }
    }
}
