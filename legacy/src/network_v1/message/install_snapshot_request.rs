use std::fmt;

use openraft::RaftTypeConfig;
use openraft::SnapshotId;
use openraft::type_config::alias::SnapshotMetaOf;
use openraft::type_config::alias::VoteOf;
use openraft_macros::since;

/// An RPC sent by the Raft leader to send chunks of a snapshot to a follower (§7).
#[since(version = "0.10.0", change = "added `snapshot_id`, moved out of `SnapshotMeta`")]
#[since(version = "0.10.0", change = "moved from `openraft::raft::InstallSnapshotRequest`")]
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct InstallSnapshotRequest<C>
where C: RaftTypeConfig
{
    /// The leader's current vote.
    pub vote: VoteOf<C>,

    /// Metadata of a snapshot: last_log_id, membership, etc.
    pub meta: SnapshotMetaOf<C>,

    /// To identify a snapshot when transferring.
    ///
    /// Caveat: even when two snapshots are built with the same `last_log_id`,
    /// they could still be different in bytes.
    pub snapshot_id: SnapshotId,

    /// The byte offset where this chunk of data is positioned in the snapshot file.
    pub offset: u64,
    /// The raw bytes of the snapshot chunk, starting at `offset`.
    pub data: Vec<u8>,

    /// Will be `true` if this is the last chunk in the snapshot.
    pub done: bool,
}

impl<C> fmt::Display for InstallSnapshotRequest<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "InstallSnapshotRequest {{ vote:{}, meta:{}, snapshot_id:{}, offset:{}, len:{}, done:{} }}",
            self.vote,
            self.meta,
            self.snapshot_id,
            self.offset,
            self.data.len(),
            self.done
        )
    }
}
