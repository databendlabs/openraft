use anyhow::Context;
use dir_transfer::DirFrame;
use dir_transfer::DirFrameProducer;
use openraft::async_runtime::WatchReceiver;
use raft_kv_rocksdb::typ::RaftError;
use raft_kv_rocksdb::typ::Snapshot;
use raft_kv_rocksdb::typ::SnapshotMeta;
use raft_kv_rocksdb::typ::SnapshotResponse;
use raft_kv_rocksdb::typ::Vote;
use serde::Serialize;

use crate::node::Node;

/// Captured wire messages that can be fragmented, interrupted, corrupted, or retried.
#[derive(Clone)]
pub struct SnapshotTransfer {
    pub vote: Vote,
    pub meta: SnapshotMeta,
    pub frames: Vec<DirFrame>,
}

impl SnapshotTransfer {
    /// Capture a real checkpoint as directory frames using the requested chunk size.
    pub async fn new(node: &Node, snapshot: &Snapshot, chunk_size: usize) -> anyhow::Result<Self> {
        let metrics = node.raft().metrics();
        let vote = metrics.borrow_watched().vote;
        let mut sender = DirFrameProducer::new(snapshot.snapshot.db_path(), chunk_size)?;
        let mut frames = Vec::new();
        while let Some(frame) = sender.next_frame().await? {
            frames.push(frame);
        }
        Ok(Self {
            vote,
            meta: snapshot.meta.clone(),
            frames,
        })
    }

    /// Encode the vote and metadata followed by each frame as separate wire messages.
    pub fn messages(&self) -> anyhow::Result<Vec<Vec<u8>>> {
        let start = (self.vote, &self.meta);
        let start = encode(&start)?;
        let mut messages = vec![start];
        for frame in &self.frames {
            let message = encode(frame)?;
            messages.push(message);
        }
        Ok(messages)
    }

    /// Encode a complete snapshot request body.
    pub fn body(&self) -> anyhow::Result<Vec<u8>> {
        let messages = self.messages()?;
        let bytes = messages.into_iter().flatten();
        Ok(bytes.collect())
    }

    /// Install the captured checkpoint through the production HTTP endpoint.
    pub async fn send(&self, node: &Node) -> anyhow::Result<SnapshotResponse> {
        let bytes = self.body()?;
        let body = reqwest::Body::from(bytes);
        let response = node.post_snapshot(body).await?;
        let response = response.error_for_status()?;
        let result: Result<SnapshotResponse, RaftError> = response.json().await?;
        let response = result.context("Raft accepts snapshot request")?;
        Ok(response)
    }
}

fn encode<T>(message: &T) -> anyhow::Result<Vec<u8>>
where T: Serialize {
    let payload = rmp_serde::to_vec(message)?;
    let len = u32::try_from(payload.len())?;
    let prefix = len.to_be_bytes();
    let mut message = prefix.to_vec();
    message.extend(payload);
    Ok(message)
}
