use std::fmt::Debug;

use super::LogId;
use super::StoredMembership;
use crate::compat::Upgrade;

/// v0.7 compatible SnapshotMeta.
///
/// SnapshotMeta can not be upgraded, an old snapshot should be discarded and a new one should be
/// re-built.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SnapshotMeta {
    pub last_log_id: Option<LogId>,

    pub last_membership: Option<StoredMembership>,

    pub snapshot_id: crate::SnapshotId,
}

impl Upgrade<crate::SnapshotMeta<u64, crate::EmptyNode>> for or07::SnapshotMeta {
    fn upgrade(self) -> crate::SnapshotMeta<u64, crate::EmptyNode> {
        unimplemented!("can not upgrade SnapshotMeta")
    }
    fn try_upgrade(self) -> Result<crate::SnapshotMeta<u64, crate::EmptyNode>, (Self, &'static str)> {
        Err((self, "v07 snapshot meta does not contain membership to upgrade"))
    }
}

impl Upgrade<crate::SnapshotMeta<u64, crate::EmptyNode>> for SnapshotMeta {
    fn upgrade(self) -> crate::SnapshotMeta<u64, crate::EmptyNode> {
        unimplemented!("can not upgrade SnapshotMeta")
    }
    fn try_upgrade(self) -> Result<crate::SnapshotMeta<u64, crate::EmptyNode>, (Self, &'static str)> {
        if self.last_membership.is_none() {
            Err((self, "v07 snapshot meta does not contain membership to upgrade"))
        } else {
            Ok(crate::SnapshotMeta {
                last_log_id: self.last_log_id.map(|lid| lid.upgrade()),
                last_membership: self.last_membership.unwrap().upgrade(),
                snapshot_id: self.snapshot_id,
            })
        }
    }
}
