//! The snapshot: a serialized [`EzApp`](crate::EzApp) and where in the log it stands

use std::io::Cursor;

use openraft::BasicNode;
use openraft::Snapshot;
use openraft::SnapshotMeta;

use crate::entry::EzCommittedLeaderId;

/// Snapshot data type: the serialized state machine bytes
pub type EzSnapshotData = Cursor<Vec<u8>>;

/// Snapshot metadata type alias
///
/// Points to OpenRaft's `SnapshotMeta` for full compatibility.
pub type EzSnapshotMeta = SnapshotMeta<EzCommittedLeaderId, u64, BasicNode>;

/// Snapshot type alias
///
/// Points to OpenRaft's `Snapshot` for full compatibility.
pub type EzSnapshot = Snapshot<EzCommittedLeaderId, u64, BasicNode, EzSnapshotData>;
