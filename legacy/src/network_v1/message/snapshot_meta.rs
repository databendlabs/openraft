use std::fmt;

use display_more::DisplayOptionExt;
use openraft::RaftTypeConfig;
use openraft::SnapshotId;
use openraft::type_config::alias::LogIdOf;
use openraft::type_config::alias::SnapshotMetaOf;
use openraft::type_config::alias::SnapshotSignatureOf;
use openraft::type_config::alias::StoredMembershipOf;
use openraft_macros::since;

/// The metadata of a snapshot in the chunked v1 snapshot protocol.
///
/// This is the 0.9 `openraft::SnapshotMeta`, kept with its exact 0.9 serialized layout —
/// `snapshot_id` included — so that [`InstallSnapshotRequest`] stays wire-compatible with 0.9
/// peers: a 0.9 receiver reads the transfer id from this position.
///
/// The 0.10 [`SnapshotMeta`](openraft::storage::SnapshotMeta) no longer carries the id, because
/// it identifies a transfer rather than a snapshot; see its compatibility note. [`Self::new()`]
/// combines that metadata with a transfer id on the sending side, and [`Self::into_meta()`]
/// drops the id again on the receiving side.
///
/// [`InstallSnapshotRequest`]: crate::network_v1::InstallSnapshotRequest
#[since(
    version = "0.10.0",
    change = "split from `openraft::storage::SnapshotMeta` to keep the 0.9 wire layout"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct SnapshotMeta<C>
where C: RaftTypeConfig
{
    /// Log entries up to which this snapshot includes, inclusive.
    pub last_log_id: Option<LogIdOf<C>>,

    /// The last applied membership config.
    pub last_membership: StoredMembershipOf<C>,

    /// To identify a snapshot when transferring.
    ///
    /// Caveat: even when two snapshots are built with the same `last_log_id`, they still could
    /// be different in bytes.
    pub snapshot_id: SnapshotId,
}

impl<C> SnapshotMeta<C>
where C: RaftTypeConfig
{
    /// Combine a snapshot's metadata with the id of the transfer it is sent in.
    pub fn new(meta: SnapshotMetaOf<C>, snapshot_id: SnapshotId) -> Self {
        Self {
            last_log_id: meta.last_log_id,
            last_membership: meta.last_membership,
            snapshot_id,
        }
    }

    /// The snapshot's own metadata, without the transfer id.
    pub fn into_meta(self) -> SnapshotMetaOf<C> {
        SnapshotMetaOf::<C> {
            last_log_id: self.last_log_id,
            last_membership: self.last_membership,
        }
    }

    /// Get the signature of this snapshot metadata for comparison and identification.
    ///
    /// The transfer id is not part of it: a signature identifies the snapshot, and the id
    /// identifies a transfer session.
    pub fn signature(&self) -> SnapshotSignatureOf<C> {
        self.clone().into_meta().signature()
    }
}

impl<C> fmt::Display for SnapshotMeta<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{snapshot_id: {}, last_log:{}, last_membership: {}}}",
            self.snapshot_id,
            self.last_log_id.display(),
            self.last_membership
        )
    }
}

#[cfg(test)]
mod tests {

    #[cfg(feature = "serde")]
    #[test]
    fn test_snapshot_meta_serde() {
        use std::collections::BTreeSet;

        use openraft::Membership;
        use openraft::StoredMembership;
        use openraft::testing::log_id;

        use super::SnapshotMeta;
        use crate::testing::TestConfig;

        let meta = SnapshotMeta::<TestConfig> {
            last_log_id: Some(log_id::<TestConfig>(1, 2, 3)),
            last_membership: StoredMembership::new(
                Some(log_id::<TestConfig>(4, 5, 6)),
                Membership::new_with_defaults(vec![BTreeSet::from([1, 2])], []),
            ),
            snapshot_id: "1-2-3-4".to_string(),
        };

        // The exact 0.9 `SnapshotMeta` layout, with a meaningful `snapshot_id` third field.
        // The field count and order are the compatibility contract with 0.9 peers under
        // positional formats (bincode, postcard), which encode a struct as a bare sequence.
        let want = r#"{"last_log_id":{"leader_id":{"term":1,"node_id":2},"index":3},"last_membership":{"log_id":{"leader_id":{"term":4,"node_id":5},"index":6},"membership":{"configs":[[1,2]],"nodes":{"1":{"addr":"localhost"},"2":{"addr":"localhost"}}}},"snapshot_id":"1-2-3-4"}"#;

        assert_eq!(want, serde_json::to_string(&meta).unwrap());
        assert_eq!(meta, serde_json::from_str::<SnapshotMeta<TestConfig>>(want).unwrap());
    }
}
