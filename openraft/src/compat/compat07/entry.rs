use std::fmt::Debug;

use super::EntryPayload;
use super::LogId;
use crate::compat::Upgrade;

/// v0.7 compatible Entry.
///
/// To load from either v0.7 or the latest format data and upgrade it to the latest type:
/// ```ignore
/// let x:openraft::Entry = serde_json::from_slice::<compat07::Entry>(&serialized_bytes)?.upgrade()
/// ```
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(bound = "")]
pub struct Entry<C: crate::RaftTypeConfig> {
    pub log_id: LogId,
    pub payload: EntryPayload<C>,
}

impl<C> Upgrade<crate::Entry<C>> for or07::Entry<C::D>
where
    C: crate::RaftTypeConfig<NodeId = u64, Node = crate::EmptyNode>,
    <C as crate::RaftTypeConfig>::D: or07::AppData + Debug,
{
    fn upgrade(self) -> crate::Entry<C> {
        let log_id = self.log_id.upgrade();
        let payload = self.payload.upgrade();
        crate::Entry { log_id, payload }
    }
}

impl<C: crate::RaftTypeConfig<NodeId = u64, Node = crate::EmptyNode>> Upgrade<crate::Entry<C>> for Entry<C> {
    fn upgrade(self) -> crate::Entry<C> {
        crate::Entry {
            log_id: self.log_id.upgrade(),
            payload: self.payload.upgrade(),
        }
    }
}
