/// Id of a snapshot stream.
///
/// Every time a snapshot is created, it is assigned with a globally unique id.
/// Such an id is used by followers to distinguish different install-snapshot streams.
pub type SnapshotId = String;
