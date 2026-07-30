//! Errors reported by the v1 chunk-based snapshot RPC.

mod install_snapshot_error;
mod snapshot_mismatch;
mod snapshot_segment_id;

pub use install_snapshot_error::InstallSnapshotError;
pub use snapshot_mismatch::SnapshotMismatch;
pub use snapshot_segment_id::SnapshotSegmentId;
