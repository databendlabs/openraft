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

    /// The offset of this segment in the entire snapshot data.
    pub offset: u64,
}

impl<D: ToString> From<(D, u64)> for SnapshotSegmentId {
    fn from(v: (D, u64)) -> Self {
        SnapshotSegmentId {
            id: v.0.to_string(),
            offset: v.1,
        }
    }
}

impl Display for SnapshotSegmentId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}+{}", self.id, self.offset)
    }
}
