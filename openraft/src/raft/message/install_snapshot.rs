use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Debug;
use std::fmt::Display;
use std::io::Cursor;

use anyerror::AnyError;
use async_trait::async_trait;
use derive_more::Display;

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

impl<C: RaftTypeConfig> Debug for InstallSnapshotRequest<C>
where C::D: Debug
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstallSnapshotRequest")
            .field("vote", &self.vote)
            .field("meta", &self.meta)
            .finish()
    }
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

#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct ExampleManifest {
    pub chunks: BTreeSet<ExampleChunkId>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Display)]
#[display(fmt = "(offset: {}, len: {})", offset, len)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct ExampleChunkId {
    pub offset: usize,
    pub len: usize,
}

#[derive(Clone)]
pub struct ExampleSnapshot {
    pub chunk_len: usize,
    pub data: Vec<u8>,
}

impl ExampleSnapshot {
    pub fn into_inner(self) -> Vec<u8> {
        self.data
    }

    pub fn get_ref(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Display)]
#[display(fmt = "{}", chunk_id)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct ExampleSnapshotChunk {
    pub chunk_id: ExampleChunkId,
    pub data: Vec<u8>,
}

impl SnapshotChunk for ExampleSnapshotChunk {
    type ChunkId = ExampleChunkId;

    fn id(&self) -> Self::ChunkId {
        self.chunk_id.clone()
    }
}

impl SnapshotManifest for ExampleManifest {
    type Iter = std::collections::btree_set::IntoIter<ExampleChunkId>;
    type ChunkId = ExampleChunkId;

    fn chunks_to_send(&self) -> Self::Iter {
        self.chunks.clone().into_iter()
    }

    fn receive(&mut self, c: &Self::ChunkId) -> Result<bool, AnyError> {
        Ok(self.chunks.remove(c))
    }

    fn is_complete(&self) -> bool {
        self.chunks.is_empty()
    }
}

#[async_trait]
impl SnapshotData for ExampleSnapshot {
    type Chunk = ExampleSnapshotChunk;
    type ChunkId = ExampleChunkId;
    type Manifest = ExampleManifest;

    async fn manifest(&self) -> Self::Manifest {
        let chunks: BTreeSet<_> = self
            .data
            .as_slice()
            .chunks(self.chunk_len)
            .enumerate()
            .map(|(i, c)| ExampleChunkId {
                offset: i * self.chunk_len,
                len: c.len(),
            })
            .collect();

        ExampleManifest { chunks }
    }

    async fn get_chunk(&self, id: &Self::ChunkId) -> Result<Self::Chunk, AnyError> {
        Ok(ExampleSnapshotChunk {
            chunk_id: id.clone(),
            data: self.data[id.offset..(id.offset + id.len)].to_vec(),
        })
    }

    async fn receive(&mut self, c: Self::Chunk) -> Result<(), AnyError> {
        if self.data.len() < (c.chunk_id.offset + c.chunk_id.len) {
            self.data.extend_from_slice(&vec![0; (c.chunk_id.offset + c.chunk_id.len) - self.data.len()]);
        }

        let _: Vec<_> = self.data.splice(c.chunk_id.offset..(c.chunk_id.offset + c.chunk_id.len), c.data).collect();

        Ok(())
    }
}

impl<'a> From<&'a [u8]> for ExampleSnapshot {
    fn from(value: &'a [u8]) -> Self {
        Self {
            chunk_len: 1024,
            data: value.to_vec(),
        }
    }
}

impl From<Cursor<Vec<u8>>> for ExampleSnapshot {
    fn from(value: Cursor<Vec<u8>>) -> Self {
        Self {
            chunk_len: 1024,
            data: value.into_inner(),
        }
    }
}
