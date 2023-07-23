use std::fmt::Debug;
use std::fmt::Display;

use anyerror::AnyError;
use async_trait::async_trait;

use crate::type_config::RTCSnapshotChunk;
use crate::type_config::RTCSnapshotChunkId;
use crate::type_config::RTCSnapshotManifest;
use crate::MessageSummary;
use crate::NodeId;
use crate::OptionalSerde;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;
use crate::Vote;

/// An RPC sent by the Raft leader to send chunks of a snapshot to a follower (§7).
#[derive(Clone)]
#[derive(PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct InstallSnapshotRequest<C: RaftTypeConfig> {
    pub vote: Vote<C::NodeId>,

    /// Metadata of a snapshot: snapshot_id, last_log_ed membership etc.
    pub meta: SnapshotMeta<C::NodeId, C::Node>,

    /// The raw bytes of the snapshot chunk, starting at `offset`.
    pub data: InstallSnapshotData<C>,
}

/// An RPC sent by the Raft leader to send chunks of a snapshot to a follower (§7).
#[derive(Clone)]
#[derive(PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum InstallSnapshotData<C: RaftTypeConfig> {
    Manifest(RTCSnapshotManifest<C>),
    Chunk(RTCSnapshotChunk<C>),
}

impl<C: RaftTypeConfig> InstallSnapshotData<C> {
    pub fn chunk_id(&self) -> Option<RTCSnapshotChunkId<C>> {
        match self {
            Self::Manifest(_) => None,
            Self::Chunk(c) => Some(c.id()),
        }
    }

    pub fn is_manifest(&self) -> bool {
        match self {
            InstallSnapshotData::Manifest(_) => true,
            InstallSnapshotData::Chunk(_) => false,
        }
    }
}

pub trait SnapshotManifest: Clone + Send + Sync + Default + PartialEq + OptionalSerde {
    type Iter: Iterator<Item = Self::ChunkId> + Send + Sync;
    type ChunkId;

    // Get a list of the remaining chunks of the snapshot to send
    fn chunks_to_send(&self) -> Self::Iter;

    // Apply a received chunk to the manifest. This removes it from the list of chunks that need to
    // be sent/received
    // Returns if true if this chunk was not sent/received before. False if it has been seen
    // before.
    fn receive(&mut self, c: &Self::ChunkId) -> Result<bool, AnyError>;

    // Return if the snapshot has received all chunks
    fn is_complete(&self) -> bool;
}

#[async_trait]
pub trait SnapshotData: Send + Sync {
    type ChunkId: Eq + PartialEq + Send + Sync + Display + Debug + OptionalSerde + 'static;
    type Chunk: SnapshotChunk<ChunkId = Self::ChunkId>;
    type Manifest: SnapshotManifest<ChunkId = Self::ChunkId>;

    // Generate the manifest for this snapshot. The manifest should be able to keep track of all
    // the chunks to send or receive
    async fn manifest(&self) -> Self::Manifest;

    // Get the chunk to be sent to the follower
    async fn get_chunk(&self, id: &Self::ChunkId) -> Result<Self::Chunk, AnyError>;

    // Receive the chunk sent to this node and apply it.
    async fn receive(&mut self, c: Self::Chunk) -> Result<(), AnyError>;
}

pub trait SnapshotChunk: Clone + PartialEq + Send + Sync + OptionalSerde {
    type ChunkId;

    fn id(&self) -> Self::ChunkId;
}

impl<C: RaftTypeConfig> MessageSummary<InstallSnapshotRequest<C>> for InstallSnapshotRequest<C> {
    fn summary(&self) -> String {
        format!("vote={}, meta={}", self.vote, self.meta,)
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
