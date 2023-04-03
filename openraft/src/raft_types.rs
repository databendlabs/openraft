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

// An update action with option to update with some value or just leave it as is.
#[derive(Debug, Clone, PartialOrd, PartialEq, Eq)]
pub enum Update<T> {
    Update(T),
    AsIs,
}

/// Describes the need to update some aspect of the metrics.
#[derive(Debug, Clone)]
#[derive(Default)]
#[derive(PartialEq, Eq)]
pub(crate) struct MetricsChangeFlags {
    /// Replication state changes. E.g., adding/removing replication, progress update.
    pub(crate) replication: bool,

    /// Local data changes. Such as vote, log, snapshot.
    pub(crate) local_data: bool,

    /// State related to cluster: server state, leader, membership etc.
    pub(crate) cluster: bool,
}

impl MetricsChangeFlags {
    pub(crate) fn changed(&self) -> bool {
        self.replication || self.local_data || self.cluster
    }

    pub(crate) fn reset(&mut self) {
        self.replication = false;
        self.local_data = false;
        self.cluster = false;
    }

    /// Includes state of replication to other nodes.
    pub(crate) fn set_replication_changed(&mut self) {
        self.replication = true
    }

    /// Includes raft log, snapshot, state machine etc.
    pub(crate) fn set_data_changed(&mut self) {
        self.local_data = true
    }

    /// Includes node role, membership config, leader node etc.
    pub(crate) fn set_cluster_changed(&mut self) {
        self.cluster = true
    }
}
