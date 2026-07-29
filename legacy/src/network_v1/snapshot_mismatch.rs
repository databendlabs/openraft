use openraft_macros::since;

use crate::network_v1::SnapshotSegmentId;

/// Error indicating a snapshot segment ID mismatch.
#[since(version = "0.10.0", change = "moved from `openraft::errors::SnapshotMismatch`")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("snapshot segment id mismatch, expect: {expect}, got: {got}")]
pub struct SnapshotMismatch {
    /// The expected snapshot segment ID.
    pub expect: SnapshotSegmentId,
    /// The actual snapshot segment ID received.
    pub got: SnapshotSegmentId,
}
