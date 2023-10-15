use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;

/// Id of a snapshot stream.
///
/// Everytime a snapshot is created, it is assigned with a globally unique id.
/// Such an id is used by followers to distinguish different install-snapshot streams.
pub type SnapshotId = String;

/// The identity of a segment of a snapshot.
#[derive(Debug, Default, Clone, PartialOrd, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct SnapshotSegmentId {
    /// The unique identifier of the snapshot stream.
    pub id: SnapshotId,
}

impl Display for SnapshotSegmentId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id)
    }
}
