use crate::raft::ExampleSnapshot;
use crate::RaftTypeConfig;
use crate::TokioRuntime;

/// Trivial Raft type config for Engine related unit test.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct UTConfig {}
impl RaftTypeConfig for UTConfig {
    type D = ();
    type R = ();
    type NodeId = u64;
    type Node = ();
    type Entry = crate::Entry<UTConfig>;
    type SnapshotData = ExampleSnapshot;
    type AsyncRuntime = TokioRuntime;
}

#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct VecManifest {
    pub chunks: BTreeSet<VecChunkId>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Display)]
#[display(fmt = "(offset: {})", offset)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct VecChunkId {
    pub offset: usize,
    pub len: usize,
}

#[derive(Clone)]
pub(crate) struct VecSnapshot {
    pub len: usize,
    pub data: Vec<u8>,
}

#[derive(Clone, PartialEq, Eq, Debug, Display)]
#[display(fmt = "{}", chunk_id)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub(crate) struct VecSnapshotChunk {
    pub chunk_id: VecChunkId,
    pub data: Vec<u8>,
}

impl SnapshotChunk for VecSnapshotChunk {
    type ChunkId = VecChunkId;

    fn id(&self) -> Self::ChunkId {
        self.chunk_id.clone()
    }
}

impl SnapshotManifest for VecManifest {
    type Iter = std::collections::btree_set::IntoIter<VecChunkId>;
    type ChunkId = VecChunkId;

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
impl SnapshotData for VecSnapshot {
    type Chunk = VecSnapshotChunk;
    type ChunkId = VecChunkId;
    type Manifest = VecManifest;

    async fn manifest(&self) -> Self::Manifest {
        let chunks: BTreeSet<_> = self
            .data
            .as_slice()
            .chunks(self.len)
            .enumerate()
            .map(|(i, c)| VecChunkId {
                offset: i * self.len,
                len: c.len(),
            })
            .collect();

        VecManifest { chunks }
    }

    async fn get_chunk(&self, id: &Self::ChunkId) -> Result<Self::Chunk, AnyError> {
        Ok(VecSnapshotChunk {
            chunk_id: id.clone(),
            data: self.data[id.offset..(id.offset + id.len)].to_vec(),
        })
    }

    async fn receive(&mut self, c: Self::Chunk) -> Result<(), AnyError> {
        if self.data.len() < (c.chunk_id.offset + c.chunk_id.len) {
            self.data.reserve((c.chunk_id.offset + c.chunk_id.len) - self.data.len());
        }

        let _: Vec<_> = self.data.splice(c.chunk_id.offset..(c.chunk_id.offset + c.chunk_id.len), c.data).collect();

        Ok(())
    }
}

impl<'a> From<&'a [u8]> for VecSnapshot {
    fn from(value: &'a [u8]) -> Self {
        Self {
            len: 1024,
            data: value.to_vec(),
        }
    }
}
