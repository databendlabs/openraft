use anyerror::AnyError;

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
    Manifest(C::SnapshotManifest),
    Chunk(C::SnapshotChunk),
}

impl<C: RaftTypeConfig> InstallSnapshotData<C> {
    pub fn chunk(data: C::SnapshotChunk) -> Self {
        Self::Chunk(data)
    }

    pub fn manifest(manifest: C::SnapshotManifest) -> Self {
        Self::Manifest(manifest)
    }

    pub fn chunk_id(&self) -> Option<<C::SnapshotChunk as SnapshotChunk>::ChunkId> {
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

pub trait SnapshotData: Send + Sync {
    type Chunk: SnapshotChunk<ChunkId = Self::ChunkId>;
    type ChunkId;
    type Manifest: SnapshotManifest;

    // Generate the manifest for this snapshot. The manifest should be able to keep track of all
    // the chunks to send or receive
    fn manifest(&self) -> Self::Manifest;

    // Get the chunk to be sent to the follower
    fn get_chunk(&self, id: &Self::ChunkId) -> Result<Self::Chunk, AnyError>;

    // Receive the chunk sent to this node and apply it.
    fn receive(&mut self, c: Self::Chunk) -> Result<(), AnyError>;
}

pub trait SnapshotChunk: Clone + PartialEq + Send + Sync + OptionalSerde {
    type ChunkId;

    fn id(&self) -> Self::ChunkId;
}

// pub struct ChunkIter<I, C> {
//     pub iter: I,
//     _c: PhantomData<C>,
// }

// impl<I, C> ChunkIter<I, C>
// where I: Iterator<Item = C>
// {
//     pub fn new(iter: impl IntoIterator<Item = C>) -> Self {
//         Self {
//             iter: iter.into_iter(),
//             _c: PhantomData,
//         }
//     }
// }

// impl<I: Iterator> Iterator for ChunkIter<I, I::Item> {
//     type Item = I::Item;

//     fn next(&mut self) -> Option<Self::Item> {
//         self.iter.next()
//     }
// }

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
