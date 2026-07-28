/// Id of a snapshot stream.
///
/// Every time a snapshot is created, it is assigned with a globally unique id.
/// Such an id is used by followers to distinguish different install-snapshot streams.
pub type SnapshotId = String;

/// **REMOVED**: Use `openraft_legacy::network_v1::SnapshotSegmentId` instead.
///
/// The v1 chunk-based `SnapshotSegmentId` has been moved to the `openraft-legacy` crate.
///
/// # Migration
///
/// Add to `Cargo.toml`:
/// ```toml
/// [dependencies]
/// openraft-legacy = "0.10"
/// ```
///
/// Update imports:
/// ```ignore
/// use openraft_legacy::network_v1::SnapshotSegmentId;
/// ```
#[deprecated(
    since = "0.10.0",
    note = "SnapshotSegmentId has been moved to the `openraft-legacy` crate. \
            Add `openraft-legacy` to your dependencies and use \
            `openraft_legacy::network_v1::SnapshotSegmentId` instead."
)]
pub struct SnapshotSegmentId {}
