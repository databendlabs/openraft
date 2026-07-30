//! Follower-side receiving of chunk-based snapshots.
//!
//! [`ChunkedSnapshotReceiver`] is the entry point; `Streaming` and `StreamingState` hold the
//! partially received snapshot on its behalf.

mod chunked_snapshot_receiver;
mod streaming;
mod streaming_state;

pub use chunked_snapshot_receiver::ChunkedSnapshotReceiver;
pub(crate) use streaming::Streaming;
pub(crate) use streaming_state::StreamingState;
