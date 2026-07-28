use openraft_macros::since;

use crate::network_v1::SnapshotMismatch;

/// Error related to installing a snapshot with the v1 chunk-based API.
#[since(version = "0.10.0", change = "moved from `openraft::errors::InstallSnapshotError`")]
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum InstallSnapshotError {
    /// The snapshot segment offset does not match what was expected.
    #[error(transparent)]
    SnapshotMismatch(#[from] SnapshotMismatch),
}
