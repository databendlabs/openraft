use crate::compat::Compat;
use crate::compat::Upgrade;

/// v0.7 compatible LogId.
///
/// To load from either v0.7 or the latest format data and upgrade it to the latest type:
/// ```ignore
/// let x:openraft::LogId = serde_json::from_slice::<compat07::LogId>(&serialized_bytes)?.upgrade()
/// ```
pub type LogId = Compat<or07::LogId, crate::LogId<u64>>;

impl Upgrade<crate::LogId<u64>> for or07::LogId {
    fn upgrade(self) -> crate::LogId<u64> {
        let committed_leader_id = crate::CommittedLeaderId::new(self.term, 0);
        crate::LogId::new(committed_leader_id, self.index)
    }
}
