use std::fmt::Debug;

use super::LogId;
use super::Membership;
use crate::compat::Upgrade;

/// v0.7 compatible StoredMembership.
///
/// To load from either v0.7 or the latest format data and upgrade it to the latest type:
/// ```ignore
/// let x:openraft::StoredMembership = serde_json::from_slice::<compat07::StoredMembership>(&serialized_bytes)?.upgrade()
/// ```
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct StoredMembership {
    pub log_id: Option<LogId>,
    pub membership: Membership,
    #[serde(skip)]
    pub quorum_set: Option<()>,
    #[serde(skip)]
    pub voter_ids: Option<()>,
}

impl Upgrade<crate::StoredMembership<u64, crate::EmptyNode>> for or07::EffectiveMembership {
    fn upgrade(self) -> crate::StoredMembership<u64, crate::EmptyNode> {
        let membership = self.membership.upgrade();
        let log_id = self.log_id.upgrade();

        crate::StoredMembership::new(Some(log_id), membership)
    }
}

impl Upgrade<crate::StoredMembership<u64, crate::EmptyNode>> for StoredMembership {
    fn upgrade(self) -> crate::StoredMembership<u64, crate::EmptyNode> {
        crate::StoredMembership::new(self.log_id.map(|lid| lid.upgrade()), self.membership.upgrade())
    }
}
