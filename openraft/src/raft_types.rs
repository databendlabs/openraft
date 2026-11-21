/// Id of one snapshot transfer of the chunked v1 network protocol.
///
/// The v1 sender in `openraft-legacy` assigns a new id each time it starts to send a snapshot, and
/// the receiver uses it to tell one install-snapshot stream from another. Sending the same snapshot
/// again therefore uses a new id, so that the new transfer is not taken for the continuation of an
/// aborted one.
///
/// A stored snapshot has no id: [`SnapshotMeta`] identifies it by its `last_log_id`, and a storage
/// implementation does not need to generate one. See the
/// [0.9 to 0.10 upgrade guide](crate::docs::upgrade_guide::upgrade_09_10).
///
/// [`SnapshotMeta`]: crate::storage::SnapshotMeta
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
