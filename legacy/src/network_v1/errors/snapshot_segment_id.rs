use std::fmt::Display;
use std::fmt::Formatter;

use openraft::SnapshotId;
use openraft_macros::since;

/// The identity of a segment of a snapshot.
#[since(version = "0.10.0", change = "moved from `openraft::SnapshotSegmentId`")]
#[derive(Debug, Clone, PartialOrd, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct SnapshotSegmentId {
    /// The unique identifier of the snapshot stream.
    pub id: SnapshotId,

    /// The offset of this segment in the entire snapshot data.
    pub offset: u64,
}

impl<D> From<(D, u64)> for SnapshotSegmentId
where D: ToString
{
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
