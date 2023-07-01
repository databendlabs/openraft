use crate::MessageSummary;
use crate::NodeId;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;
use crate::Vote;

/// An RPC sent by the Raft leader to send chunks of a snapshot to a follower (§7).
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct InstallSnapshotRequest<C: RaftTypeConfig> {
    pub vote: Vote<C::NodeId>,

    /// Metadata of a snapshot: snapshot_id, last_log_ed membership etc.
    pub meta: SnapshotMeta<C::NodeId, C::Node>,

    /// The byte offset where this chunk of data is positioned in the snapshot file.
    pub offset: u64,
    /// The raw bytes of the snapshot chunk, starting at `offset`.
    pub data: Vec<u8>,

    /// Will be `true` if this is the last chunk in the snapshot.
    pub done: bool,
}

impl<C: RaftTypeConfig> MessageSummary<InstallSnapshotRequest<C>> for InstallSnapshotRequest<C> {
    fn summary(&self) -> String {
        format!(
            "vote={}, meta={}, offset={}, len={}, done={}",
            self.vote,
            self.meta,
            self.offset,
            self.data.len(),
            self.done
        )
    }
}

/// The response to an `InstallSnapshotRequest`.
#[derive(Debug)]
#[derive(PartialEq, Eq)]
#[derive(derive_more::Display)]
#[display(fmt = "{{vote:{}}}", vote)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct InstallSnapshotResponse<NID: NodeId> {
    pub vote: Vote<NID>,
}
